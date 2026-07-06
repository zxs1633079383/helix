# Helix 多租户本地库架构设计

> 日期：2026-07-06
> 状态：设计稿
> 适用范围：PC Tauri、Flutter FFI、host-cli、loopforge-tauri-im
> 关联约束：GOAL.md、ADR-007、HX-C001、HX-C005、HX-C006、HX-C007

## 0. 结论

之前“不同用户不同文件夹隔离”的方向是对的，但粒度不够。Helix 应该采用：

**一个逻辑本地身份一个 store 文件夹，一个 store 文件夹一个 `tenant.db`。**

这里的“逻辑本地身份”不是单纯 `userId`，而是：

```
LocalStoreIdentity =
  app_namespace
  + backend_scope(api/ws/env)
  + company_id
  + account_id(userId/cookieId)
  + tenant_hint
  + schema_family
```

落地形态：

```
<platform_app_data>/data/<local_store_key>/
  manifest.json
  tenant.db
  tenant.db-wal
  tenant.db-shm
```

核心取舍：

- **保留物理文件夹隔离**：不同账号、不同环境、不同公司不共享 SQLite 文件。
- **不在所有 IM 表上加 `tenant_id`**：避免改现网 678 `tenant.db` 兼容 schema，避免每个 query 都带漏过滤风险。
- **路径派生收口到 `helix-driver-host` 或平台壳共享 helper**：PC、FFI、host-cli、loopforge 不能各自拼路径。
- **`helix-core` 和 `helix-im` 不认识目录/租户路径规则**：core 仍只吐 `StorageOp`，IM schema 仍作为数据交给 driver 执行。

一句话：**我们不是“userId 文件夹隔离”，而是“store identity 文件夹隔离”。**

## 1. 当前代码事实

### 1.1 存储单一真源已经在 driver-host

`docs/harness/decisions.md` 的 ADR-007 已拍板：`helix-driver-host` 是 PC Tauri 与 Flutter FFI 共享 Rust host driver 层，SQLite schema/migration、rusqlite `Storage` 实现、`StorageOp -> PortOutcome`、rows 编解码都在这里统一维护。

这意味着多租户本地库不能在 native 一套、FFI 一套、host-cli 一套。路径派生也应该收敛到共享层或共享 helper，否则又会出现“双端同逻辑漂移”。

### 1.2 HostStorage 当前只接受 db_url

当前 `HostStorage::open_sqlite_url(db_url)` 的职责是：

- 解析 `sqlite:` URL。
- 打开写连接和只读连接池。
- 设 WAL / busy_timeout / migration。
- 对外兑现 `Storage` port。

它不负责“这个用户应该开哪个库”。这是合理边界。路径派生属于 driver/platform shell 的启动配置，不属于 `Storage` trait，也不属于 core。

### 1.3 host-cli 已经有一版临时派生

`crates/helix-host-cli/src/host_env.rs` 当前已有：

```
~/Library/Application Support/com.jinqidongli.cses.dev/data/<tenant>/tenant.db
```

`<tenant>` 的来源是：

```
identity.tenant > cookie_id(userId) > "default"
```

这证明“文件夹隔离”已经被实际使用过，但它仍有两个问题：

- `tenant` 名字容易误导，实际可能是 userId，也可能是配置 tenant。
- 只看 `tenant/userId` 不够区分 pre/prod、公司、后端 scope。

### 1.4 FFI 当前靠 HELIX_DB 注入

`crates/helix-driver-ffi/src/api.rs` 当前写法是：

```
HELIX_DB > 进程临时库
```

这适合作为 debug 逃生口，但不能成为生产主路径。移动端生产应该由 C++/Dart host 提供 app support dir 和登录身份，Rust 侧派生或接收明确的 `db_url`。

## 2. 问题定义

我们要解决的不是“SQLite 怎么开”，而是“哪个身份打开哪个本地库”。

必须满足：

1. 同一设备上 A 用户和 B 用户数据物理隔离。
2. 同一 userId 在 pre/prod 或不同 server scope 不混库。
3. 同一 userId 在不同 company/tenant 不混库。
4. 登录切换可预测：旧 engine drain 后关闭旧库，新 engine 打开新库。
5. loopforge、PC Tauri、Flutter FFI 路径规则一致。
6. 不破坏 `helix-core` runtime-agnostic 边界。
7. 不把 `tenant_id` 过滤散落到每个 IM query。
8. 能兼容现网/678 的 `tenant.db` 文件语义和 DataGrip 调试习惯。

## 3. 方案比较

### 3.1 方案 A：不同用户不同文件夹

形态：

```
data/<user_id>/tenant.db
```

优点：

- 简单，容易理解。
- SQLite 文件级隔离天然阻止跨用户读写。
- 不需要改 IM 表 schema。

问题：

- `user_id` 不是完整身份，同 userId 在不同后端环境可能撞库。
- 多公司、多租户、多后端 base 时无法表达。
- 目录名如果直接用 userId，调试友好但隐私较差，也要防路径注入。
- PC、FFI、loopforge 若各自实现，规则会漂。

结论：

**方向可保留，但不能停在 userId。**

### 3.2 方案 B：单 DB，多表加 tenant_id

形态：

```
data/tenant.db
message(tenant_id, ...)
channel(tenant_id, ...)
channel_member(tenant_id, ...)
```

优点：

- 只维护一个 SQLite 文件。
- 可以跨租户做统一索引、统一清理。

问题：

- 需要改大量表 schema，与现网 678 `tenant.db` 兼容性变差。
- 每个 query/update/delete 都必须带 tenant 条件，漏一个就是跨用户数据泄漏。
- `StorageOp` 当前是 schema-agnostic 通用 spec，不能让 driver 自动理解 tenant 语义。
- 热路径会多复合索引和额外绑定，收益不明显。
- 回滚、删除、调试都更复杂。

结论：

**不采用。** 这个方案把隔离从文件系统边界降级为“每条 SQL 都别写错”，风险更高。

### 3.3 方案 C：按 LocalStoreIdentity 文件夹隔离，DB 内不加 tenant_id

形态：

```
data/<local_store_key>/tenant.db
data/<local_store_key>/manifest.json
```

优点：

- 文件级硬隔离。
- 不改 IM 业务 schema。
- 同 userId 在不同后端/company 不混库。
- 路径派生可测试、可审计、可跨端复用。
- 适配现有 `HostStorage`，只是在 open 之前多一步 locate。

问题：

- 需要定义 `LocalStoreIdentity` 和稳定 key。
- 登录态不足时不能随便落正式库，需要进入 transient store 或拒绝启动 IM。
- 多窗口同一 store 要有单引擎复用策略，不能开两个独立 WS engine 抢同一身份。

结论：

**采用。**

## 4. 领域模型

### 4.1 LocalStoreIdentity

建议定义在 driver-host 的本地存储配置模块，或放在一个共享 host helper 中：

```rust
pub struct LocalStoreIdentity {
    pub app_namespace: String,
    pub backend_scope: BackendScope,
    pub company_id: String,
    pub account_id: String,
    pub tenant_hint: Option<String>,
    pub schema_family: String,
}

pub struct BackendScope {
    pub environment: String,
    pub api_base_url: String,
    pub ws_url: String,
}
```

字段含义：

| 字段 | 用途 | 是否参与 key |
|---|---|---|
| `app_namespace` | 区分 cses dev/prod、loopforge、测试壳 | 是 |
| `backend_scope.environment` | pre/prod/local/e2e | 是 |
| `backend_scope.api_base_url/ws_url` | 防同环境名但后端地址不同 | 是，先 normalize |
| `company_id` | 同用户多公司隔离 | 是 |
| `account_id` | userId/cookieId | 是 |
| `tenant_hint` | 兼容现有 tenant 字段，不作为唯一身份 | 是，允许空 |
| `schema_family` | `im-v1`，未来 schema 大迁移可分家 | 是 |

不建议参与 key：

- `device_id`：同一 app 在同一设备上本来就是一个 app data sandbox；device_id 换了不应导致用户数据消失。
- token/header：凭据会刷新，不是本地库身份。
- display name/org name/dept name：展示字段会变，不是身份。

### 4.2 LocalStoreKey

`LocalStoreKey` 是目录名，不直接等于 userId。

推荐：

```
local_store_key = "s_" + hex(sha256(canonical_json(LocalStoreIdentity)))[0..24]
```

理由：

- 稳定。
- 防路径注入。
- 避免 raw userId/companyId 直接暴露在目录名。
- 长度可控，跨平台安全。

为了可调试，在同目录写 `manifest.json`：

```json
{
  "version": 1,
  "localStoreKey": "s_0123456789abcdef01234567",
  "appNamespace": "com.jinqidongli.cses.dev",
  "environment": "pre",
  "apiBaseUrl": "http://127.0.0.1:8066/api/cses",
  "wsUrl": "ws://127.0.0.1:8066/api/v4/websocket",
  "companyId": "64118eeb...",
  "accountId": "444",
  "tenantHint": "444",
  "schemaFamily": "im-v1",
  "createdAtMs": 1783324893000,
  "lastOpenedAtMs": 1783324893000
}
```

manifest 不能写 token、cookie、authorization、真实密码类字段。

### 4.3 LocalStoreLocator

建议接口：

```rust
pub struct LocalStoreLocator {
    pub base_dir: PathBuf,
}

pub struct LocalStorePaths {
    pub key: LocalStoreKey,
    pub dir: PathBuf,
    pub db_path: PathBuf,
    pub db_url: String,
    pub manifest_path: PathBuf,
}

impl LocalStoreLocator {
    pub fn resolve(&self, identity: &LocalStoreIdentity) -> Result<LocalStorePaths, LocalStoreError>;
}
```

职责：

- normalize identity。
- 计算 key。
- 创建目录。
- 写入或更新 manifest。
- 返回 `sqlite:<path>?mode=rwc`。

不负责：

- 打开 SQLite 连接。
- 执行 IM schema。
- 做登录/刷新 token。
- 判断业务租户权限。

## 5. 目录布局

推荐统一布局：

```
<platform_app_data>/
  data/
    registry.json
    s_0123456789abcdef01234567/
      manifest.json
      tenant.db
      tenant.db-wal
      tenant.db-shm
      traces/
      fixtures/
```

说明：

- `data/` 保持与当前 host-cli 方向一致，降低迁移成本。
- `tenant.db` 文件名保留，继续兼容现有调试口径。
- `registry.json` 只做索引，不能作为打开库的权威。权威仍是 identity -> key 的纯函数。
- WAL/SHM 与 DB 同目录，便于整体备份/删除。
- `traces/fixtures/` 可选，便于 loopforge golden frame 按 store 收集。

## 6. 启动和切换流程

### 6.1 首次登录

```
login/session ready
  -> 组 LocalStoreIdentity
  -> LocalStoreLocator.resolve(identity)
  -> HostStorage::open_sqlite_url(paths.db_url)
  -> execute IM_SCHEMA
  -> register ImModule
  -> start engine
```

如果 `account_id` 或 `company_id` 缺失：

- 真生产：不启动 IM 正式库，返回配置错误。
- 测试/未登录预览：可进入 transient store，但必须在 manifest 标 `transient=true`，且不得 sync 真服务端。

### 6.2 切换账号

```
old engine graceful drain
  -> destroy / join pump
  -> drop old HostStorage
  -> resolve new identity
  -> open new tenant.db
  -> start new engine
```

不允许在同一个 engine 实例里“热切 DB”。原因：

- engine 内有 channel gate、cursor、pending send、transport 等内存态。
- 只换 SQLite 会造成内存态和库身份不一致。
- 正确动作是销毁旧 engine，打开新 engine。

### 6.3 登出

默认策略：

- 清 header/token/auth registry。
- drain engine。
- 保留本地库，用于下次同账号快速恢复。

可选策略：

- 用户选择“清除本机数据”时删除整个 store dir。
- 删除前必须确保 engine 已 destroy，避免 WAL 未收尾。

### 6.4 多窗口同账号

同一进程内：

- 同一个 `LocalStoreKey` 应复用一个 engine handle。
- 多窗口只订阅同一个 event bus，不各自开 WS/pump。

跨进程：

- SQLite WAL 能处理多连接，但两个 engine 同时持有 WS 和内存态会造成业务重复消费。
- 当前阶段不支持同 store 多 engine 写。后续如果 PC 需要多进程，要加 store lock 或单实例 broker。

## 7. 各端落点

### 7.1 helix-driver-host

新增共享路径派生模块，作为单一规则源：

```
crates/helix-driver-host/src/local_store.rs
```

建议内容：

- `LocalStoreIdentity`
- `BackendScope`
- `LocalStoreKey`
- `LocalStoreLocator`
- `LocalStorePaths`
- path sanitize / canonical JSON / hash
- manifest read/write

`HostStorage` 仍保持只接收 `db_url`，不直接依赖 locator。这样 Storage 仍聚焦 SQLite 连接和 `StorageOp` 兑现。

### 7.2 host-cli

把 `crates/helix-host-cli/src/host_env.rs::resolve_db_path` 改为调用 `LocalStoreLocator`。

保留：

```
runtime.db 显式覆盖 > LocalStoreLocator 派生 > /tmp 兜底
```

区别：

- 不再只用 `identity.tenant > cookie_id` 当目录名。
- `connection.api_base/ws_url`、`identity.company_id` 也参与 key。

### 7.3 PC Tauri

PC Tauri 的 base dir 由 Tauri `app_data_dir` 提供：

```
app.path().app_data_dir()/data/<local_store_key>/tenant.db
```

Tauri 壳负责从登录态组 `LocalStoreIdentity`：

- app namespace
- api/ws base
- companyId
- userId/cookieId
- tenant hint

Angular 不直接知道 DB 路径。Angular 只 invoke 命令、订阅 `im:__bus__`。

灰度切换 legacy/helix 时必须使用同一个 `LocalStoreIdentity`，这样 legacy 与 helix 共享同一个 `tenant.db`，回滚不丢数据。

### 7.4 Flutter FFI

FFI 生产主路径不能长期靠 `HELIX_DB`。

建议两步走：

1. 短期：C++/Dart 按同一 `LocalStoreLocator` 规则预先算出 db_url，通过现有 debug env 注入，loopforge/移动验证先跑通。
2. 正式：扩展 FFI create 配置入口，传入 app support dir 和 identity JSON，由 Rust 侧调用 locator。

正式接口方向：

```c
helix_create_with_config(config_json, config_json_len, vtable)
```

其中 config 包含：

- apiBaseUrl
- wsUrl
- appDataDir
- appNamespace
- environment
- companyId
- accountId
- tenantHint
- schemaFamily

如果不想新增 C ABI 符号，也可以把现有 create 升级为传 `HostConfig` JSON。但这会破现有 C++ 调用方，需要单独评审。

### 7.5 loopforge-tauri-im

loopforge 是首个验证场，不应继续长期使用：

```
/tmp/loopforge-im.db
```

建议改为：

```
<loopforge app data>/data/<local_store_key>/tenant.db
```

loopforge 的 real-chain report 记录：

- localStoreKey
- dbPath
- manifest 摘要
- HTTP/WS/bus/projection 证据对应的 store

这样 golden frame 回灌 Helix 时，能知道数据来自哪个本地身份。

## 8. 为什么不采用“单库 tenant_id”

本项目的核心风险不是“一个 SQLite 文件太多”，而是“业务逻辑写多份、边界漂移、可回放性下降”。

单库 tenant_id 会带来：

- 每个 `ScanSpec/GetSpec/BatchDeleteSpec` 都要额外 scope。
- 现网兼容表要加复合主键或复合索引。
- 一条漏 tenant 过滤就是跨账号读写事故。
- `helix-im` 需要到处感知 tenant scope，污染业务状态机。
- driver 层无法替业务自动补 tenant，因为 driver 不解释业务 schema。

文件夹隔离则把风险前移到启动阶段：

- 启动时算错库会被 manifest 和路径测试抓到。
- 启动后所有 SQL 都在单租户库内执行，不需要每条 SQL 再证明没漏 tenant。

这更符合 Helix 的 sans-IO 思路：**身份和路径是 host 配置，业务逻辑不背这个锅。**

## 9. 迁移策略

### 9.1 兼容已有 host-cli 路径

当前已有：

```
data/<tenant>/tenant.db
```

迁移时：

1. 如果 `runtime.db` 显式配置，完全尊重，不迁移。
2. 如果发现 legacy path 存在，且 manifest 不存在：
   - 读取当前 config 生成新 `LocalStoreIdentity`。
   - 计算 `local_store_key`。
   - 若新路径不存在，可移动目录或创建 alias manifest。
   - 若新路径已存在，不自动合并，写告警，要求人工确认。
3. 一段时间内支持 legacy resolver fallback，仅用于读老库。

### 9.2 loopforge `/tmp` 迁移

`/tmp` 库只适合临时调试，不做自动迁移。loopforge 切到新 locator 后：

- 新 real-chain 使用新库。
- 旧 `/tmp/loopforge-im.db` 作为历史调试文件，不参与正式验收。

### 9.3 schema migration

每个 `tenant.db` 独立执行 migration。

需要新增一个通用表：

```sql
CREATE TABLE IF NOT EXISTS _helix_store_meta (
  key TEXT PRIMARY KEY NOT NULL,
  value TEXT NOT NULL,
  updated_at INTEGER NOT NULL
);
```

用途：

- 记录 `local_store_key`
- 记录 `schema_family`
- 记录 `manifest_version`
- 记录上次成功迁移版本

这张表属于 driver-host 通用 meta，不属于 IM 业务 schema。

## 10. 验收标准

### 10.1 单元测试

新增 locator 测试：

- 同一 identity 多次 resolve 得到同一 key。
- 不同 `account_id` 得到不同 key。
- 相同 `account_id` 但不同 `company_id` 得到不同 key。
- 相同 `account_id` 但不同 `api_base_url/ws_url/environment` 得到不同 key。
- `../evil`、空格、中文、特殊字符不会进入目录名，只进 manifest。
- manifest 不包含 token/header。

### 10.2 集成测试

新增 storage isolation 测试：

1. 用 identity A 打开库，写 message/channel。
2. 用 identity B 打开库，scan 同表为空。
3. 再打开 identity A，数据仍在。
4. A/B 的 WAL/SHM 文件在不同目录。

### 10.3 双端验收

- host-cli resolve 路径与 driver-host locator 一致。
- PC Tauri 和 FFI 对同一 identity 得到同一相对 store key。
- FFI 不设正式身份时不落正式库。
- `cargo tree -p helix-core | grep -iE 'tokio|sqlx|reqwest|web-sys|tauri'` 仍为空。
- `git diff -- crates/helix-core` 为空。

### 10.4 loopforge 验收

real-chain report 必须包含：

```json
{
  "localStoreKey": "...",
  "dbPath": ".../data/.../tenant.db",
  "manifest": {
    "environment": "...",
    "companyId": "...",
    "accountId": "..."
  }
}
```

并证明：

- 发送消息后落的是该 `dbPath`。
- WS post、bus、projection 与同一 `localStoreKey` 相关联。

## 11. 实施切片

### Phase 1：设计收口和 host-cli 复用

文件：

- `crates/helix-driver-host/src/local_store.rs`
- `crates/helix-driver-host/src/lib.rs`
- `crates/helix-driver-host/tests/local_store_locator.rs`
- `crates/helix-host-cli/src/host_env.rs`

目标：

- 提供 LocalStoreLocator。
- host-cli 改用共享 locator。
- 保留 `runtime.db` 显式覆盖。

验证：

```
cargo test -p helix-driver-host local_store
cargo test -p helix-host-cli config
```

### Phase 2：FFI 生产配置入口

文件：

- `crates/helix-driver-ffi/src/api.rs`
- `crates/helix-driver-ffi/src/pump.rs`
- `crates/helix-driver-ffi/tests/mobile_contract_test.rs`

目标：

- 不再把 `HELIX_DB` 当生产主路径。
- 新增或升级 create config，使移动端能传 app support dir + identity。
- `HELIX_DB` 保留为 debug override。

验证：

```
cargo test -p helix-driver-ffi --test mobile_contract_test
cargo check -p helix-driver-ffi
```

### Phase 3：PC Tauri / loopforge 接入

落地仓：

- `loopforge-tauri-im`
- 后续正式 `cses-client` Tauri 壳

目标：

- loopforge 不再默认 `/tmp/loopforge-im.db`。
- report 写入 localStoreKey/dbPath/manifest。
- Tauri 灰度 legacy/helix 使用同一 store identity。

验证：

```
real-chain-uc
SQLite 查询 report 中 dbPath
HTTP / WS / bus / projection 四段证据对齐同一 localStoreKey
```

## 12. 风险和不做项

### 12.1 风险

- 身份字段不足会导致错误隔离。解决：缺 `account_id/company_id/backend_scope` 时拒绝正式 store。
- 多窗口同账号重复开 engine。解决：同进程 store registry 复用 engine。
- legacy 目录迁移误合并。解决：默认不合并两个已有 store，只告警。
- 目录 hash 不可读。解决：manifest/registry 提供可调试索引。

### 12.2 不做项

- 不做单库多 tenant_id。
- 不在 `helix-core` 增加租户概念。
- 不让 `helix-im` 拼本地路径。
- 不把 token/cookie 写进 manifest。
- 不支持同一个 store 多个独立 engine 同时消费 WS。

## 13. 最终拍板建议

采用方案 C：

**按 `LocalStoreIdentity` 派生 `LocalStoreKey`，每个 key 一个文件夹，一个 `tenant.db`。**

保留“不同用户不同文件夹”的工程直觉，但把“用户”升级为“完整本地 store 身份”：

- userId 只是 key 的一部分。
- company/backend/env 也是 key 的一部分。
- DB 内不加 tenant_id。
- 路径规则收口到共享 host 层。
- engine 生命周期按 store 切换，不热换 DB。

这条路最贴合 Helix 北极星：core 不染平台，IM 不染路径，driver-host 做平台 I/O 单一真源，PC 和移动端 parity by construction。
