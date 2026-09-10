# Helix MessageV3 Phase1–5 真实链交接

> 交接范围：`/System/Volumes/Data/workspace/angular/visual/helix`
>
> 更新时间：2026-09-04（Asia/Shanghai）
>
> 当前结论：`GREEN / PRE 全量 83/83，BLOCKED_PRECONDITION=0`

```text
AGENT_HANDOFF/v1
state: green
scope: helix-messagev3-real-chain
worktree: /System/Volumes/Data/workspace/angular/visual/helix
branch: reactor/MessageV3-helix
implementation-head: 96634ed8（Helix 业务修复 71fe8670 + real-chain harness 96634ed8）
upstream: origin/reactor/MessageV3-helix（本任务未自动 push）
gate-denominator: 83 (Phase1 32 / Phase2 16 / Phase3 15 / Phase4 12 / Phase5 8)
default-environment: pre
real-chain-status: 83/83 PASS；TEST_FAILED=0，NOT_VERIFIED=0，BLOCKED_PRECONDITION=0
```

## 0. 2026-09-04 当前推进快照

- 最终完整 PRE suite：`messagev3-1788452083888-89154`，**83/83 PASS**，总耗时
  `54.105s`；按 5 个 Phase 批次执行、每个 Phase 最多 5 个活跃频道、账号级 WS/engine/
  SQLite 复用开启，5 个 Cargo 批次均未超时。前一轮
  `messagev3-1788451617448-81346` 也为 83/83，说明最终结果不是单次偶然绿色。
- 上传域使用真实 MessageV3/Helix 媒体链：本地字节 → Java `/oss/getPresignedUrl` →
  签名 OSS PUT → Helix Local Complete → Go `posts/create` → WS → SQLite →
  NativeEventSink；6 个上传 Gate 全部通过且本轮 fixture 已清理。
- G07D 根因是 type=6 `post_read` 专属位图落库后，又被缺失 `readBits` 的 type=1
  canonical stream 覆盖为空。在线、sync/readback、gap/too-long 的 type=1 upsert 现均保护
  已有非空 `read_bits`；type=6 只有明确携带 read bits 才写专属 patch。
- G01H/G01K 的偶发终态超时由首发过度依赖 sender WS echo 引起。首次发送和 retry 现在都在
  HTTP admission 后启动有界 Go history readback；WS echo 先到时仍幂等结算，readback 不会
  提前发布 sent。
- `CHAIN-TXT-G02` 与所有 Phase cleanup 均有 watchdog 和有界释放；本轮无清理挂起。
  `cargo test -p helix-im` 全包通过；`artifacts/` 和
  `docs/Cargo私仓管理员执行Prompt.md` 仍为用户未跟踪文件，不纳入提交或删除。

## 1. 给接手人的最短结论

1. 当前 `phase1-5-real-chain.mjs --all` 的正式分母是 **83 个可执行 Gate**，不是历史客户端合同的 92 个；9 个非 Go/Helix Gate 已从执行分母移除。
2. 默认运行环境是 `pre`，Go HTTP、Go WS 和 Java API 均指向 PRE；`local` 仅在显式切换时使用。
3. 最新两个完整 PRE suite 均为 **83/83 PASS**；最终 suite 为
   `messagev3-1788452083888-89154`，`BLOCKED_PRECONDITION`、`NOT_VERIFIED` 和
   `TEST_FAILED` 全部归零。
4. 发送终态同时接受 WS echo 与有界 history readback 两条权威路径；无论哪条先完成，均须经过持久化屏障，不能把 HTTP admission 当作 sent。
5. `artifacts/messagev3-phase1-5-real-chain/` 是运行证据目录，当前保持未跟踪状态；不要把账号、Cookie、密码、DSN 或消息正文写入仓库，也不要删除其他任务产生的证据。

## 2. 范围、所有权与边界

| 层 | 当前所有者 | 本交接负责什么 | 不由本交接证明什么 |
| --- | --- | --- | --- |
| `helix-im` / `helix-core` | Helix | Command、Effect、Domain Event、投影和端口边界 | 不直接拥有 HTTP、WS、SQLite 运行时 |
| `helix-driver-native` | Helix | Native HTTP/WS/SQLite/EventSink 适配，以及真实链测试 harness | 不等于 Angular UI/DOM 验收 |
| `cses-im-server` | Go 仓库 | 真实 HTTP/WS、频道成员快照、消息持久化和推送 | 本文不修改 Go 仓库，也不替代 Go Pod 部署证明 |
| Java API | Java 旧后端 | 认证/用户或媒体前置能力（按 Gate 使用） | Java health/HTTP 200 不能单独升级为业务 PASS |
| Angular/Tauri UI | `cses-client` | UI 生命周期、DOM 和浏览器行为 | 本 runner 不产生 UI/DOM 证据 |

依赖方向必须保持：业务模块依赖 `helix-core` ports；driver 实现 ports；core 不导入
`tokio`、`sqlx`、`reqwest`、`web-sys` 或 `tauri`。真实 Gate 的结论按证据平面分开记录，
不能用 mock、静态 fixture、health 200 或单层 WS 帧替代完整业务链。

## 3. 标准真实链路

```text
runner 解析 profile
  → Go health（仅 readiness）
  → 认证 actor + observer + third
  → 真实 Go channel/create（userIds 拉入 observer、third）
  → channel/query + active member snapshot 回读
  → observer 原始 Go WS
  → Native engine + actor WS + run-owned SQLite + NativeEventSink
  → Tick::Command（例如 im_send_message）
  → Helix Effect / im:post:sending + SQLite sending
  → Go HTTP/WS post
  → Helix WS 入站、projection、PersistAtomic/PersistOk
  → im:post:received + SQLite sent read-back
  → NativeEventSink / bus correlation
  → engine、WS、SQLite、频道 fixture 全部清理
```

### 3.1 创建频道和拉人是否真的执行

需要频道 fixture 的 Gate 会执行 `create_channel_at`，不是只在内存中伪造一个 channel：

- actor 使用真实认证上下文调用 Go `/channel/create`；
- 请求体带 `userIds`，将 observer 和 third 加入同一 run-owned 频道，并使用 `forceCreate`；
- 随后通过 `/channel/query` 读回 canonical channel ID，并等待 actor、observer、third 三者都出现在 active member snapshot；
- fixture 验证失败时会尝试调用 `/channel/close`，避免把半成品频道留给下一 Gate；
- 发送 Gate 在这个频道上再启动 Helix engine、actor WS、observer WS 和 SQLite，最后走完整发送/回声/持久化链。

这不是每一个 Gate 都必然创建群；纯读、本地或不需要频道的 Gate 会使用自己的最小前置。
但涉及频道成员的 real-chain Gate 不会用 fake member snapshot 代替真实 Go 回读。

### 3.2 发送 Gate 的终态门槛

`im_send_message` 从公开 `Tick::Command` 进入。发送 Gate 至少要同时看到：

1. Helix 的 `im:post:sending`；
2. 本轮 SQLite 中 `send_status=sending` 的乐观投影；
3. 真实 Go WS `action=post`，且能按 temporary ID/correlation 精确匹配；
4. Helix 的 `im:post:received`；
5. SQLite `send_status=sent` 的 durable read-back；
6. NativeEventSink/bus 上的对应事件；
7. engine、actor/observer WS、SQLite/WAL/SHM 和远端频道均完成 run-owned 清理。

只有 health、HTTP 2xx、单个 WS frame 或一个 optimistic row 时，状态只能是
`PARTIAL`/`NOT_VERIFIED`，不能写成业务 PASS。

## 4. 当前 83 Gate 分母与功能地图

| Phase | 数量 | 领域与功能边界 |
| --- | ---: | --- |
| Phase1 | 32 | 公告、频道创建、草稿、普通 post 及其变体、定时、媒体上传/引用 |
| Phase2 | 16 | 已读/未读读取、同步、时间线与导航 |
| Phase3 | 15 | 频道生命周期/属性、频道成员增删与成员状态 |
| Phase4 | 12 | 公告操作、收藏、置顶、搜索读链 |
| Phase5 | 8 | Common 文字接龙的共享链约束、幂等/版本/失败终态，以及 Text late-join/reconcile |

当前 runner 发现到的 Gate 分组如下；精确 test filter 以 `--list` 输出为准：

```text
Phase1/announcement: MV3-G07B
Phase1/channel: MV3-G08A
Phase1/draft: MV3-G02E
Phase1/post: MV3-G01A, MV3-G01B, MV3-G01F, MV3-G01H, MV3-G01I, MV3-G01J,
  MV3-G01K, MV3-G01L, MV3-G02A, MV3-G02B, MV3-G02C, MV3-G03A, MV3-G03B,
  MV3-G05A, MV3-G05B, MV3-G05C, MV3-G05D, MV3-G05E
Phase1/schedule: MV3-G04A, MV3-G04B, MV3-G04C, MV3-G04D, MV3-G04E
Phase1/upload: MV3-G01C, MV3-G01D, MV3-G01E, MV3-G01G, MV3-G02D, MV3-G03C
Phase2/read: MV3-G06A, MV3-G06B, MV3-G06D
Phase2/sync: MV3-G13A, MV3-G13B, MV3-G13C, MV3-G13D
Phase2/timeline: MV3-G11A, MV3-G11B, MV3-G11C, MV3-G11D, MV3-G11E, MV3-G11G,
  MV3-G11H, MV3-G12A, MV3-G12B
Phase3/channel: MV3-G08B, MV3-G08C, MV3-G08D, MV3-G10A, MV3-G10B, MV3-G10C,
  MV3-G10D, MV3-G10E, MV3-G10F, MV3-G10G
Phase3/channel_member: MV3-G09A, MV3-G09B, MV3-G09C, MV3-G09D, MV3-G09E
Phase4/announcement: MV3-G07A, MV3-G07C, MV3-G07D
Phase4/bookmark: MV3-G05I, MV3-G05J, MV3-G05K
Phase4/pin: MV3-G05F, MV3-G05G, MV3-G05H
Phase4/search: MV3-G14A, MV3-G14B, MV3-G14C
Phase5/chain: CHAIN-COM-G01, CHAIN-COM-G02, CHAIN-COM-G03, CHAIN-COM-G04,
  CHAIN-COM-G05, CHAIN-COM-G06
Phase5/text_chain: CHAIN-TXT-G01, CHAIN-TXT-G02
```

## 5. 从 real-chain 执行分母移除的 9 个 Gate

这些条目会保留在客户端/生产能力合同中，但由 runner 显式排除，不能被误报为失败，
也不能伪造为 PASS。

### 5.1 `NOT_APPLICABLE`：没有独立 Go/Helix mutation

| Gate | 原因 |
| --- | --- |
| `Phase1/post/MV3-G05L` | 纯本地工具栏 zero-write；没有独立 HTTP/WS/持久化 mutation |
| `Phase2/read/MV3-G06C` | Angular active-dialog 生命周期；不是 Helix 远程写链 |
| `Phase2/read/MV3-G06E` | Angular 从既有状态派生未读角标；没有独立 Go/Helix mutation |
| `Phase2/timeline/MV3-G11F` | Angular viewer window/scroll 生命周期；不属于 native real-chain |
| `Phase2/sync/MV3-G13E` | 账号 Runtime/Renderer 生命周期与恢复调度；不属于独立业务 mutation |

### 5.2 `NOT_IMPLEMENTED`：生产能力尚未归入当前 real-chain

| Gate | 原因 |
| --- | --- |
| `Phase5/acceptance/CHAIN-ACC-G01` | 跨域 acceptance 仍是能力记录，不是当前生产 TEXT real-chain |
| `Phase5/category_chain/CHAIN-CAT-G01` | 分类模式未纳入当前生产实现 |
| `Phase5/category_chain/CHAIN-CAT-G02` | 分类选项未纳入当前生产实现 |
| `Phase5/category_chain/CHAIN-CAT-G03` | 分类统计未纳入当前生产实现 |

runner 在目录发现和 libtest filter 两层同时排除这 9 个条目，防止 `--all` 产生其副作用。

## 6. 当前证据账与状态解释

### 6.1 仓内可见的全量 PRE suite

| suite | 环境 | 选择数 | PASS | TEST_FAILED | NOT_VERIFIED | BLOCKED_PRECONDITION | 结论 |
| --- | --- | ---: | ---: | ---: | ---: | ---: | --- |
| `messagev3-1788452083888-89154` | pre | 83 | 83 | 0 | 0 | 0 | 最终全绿证据，54.105s |
| `messagev3-1788451617448-81346` | pre | 83 | 83 | 0 | 0 | 0 | 连续全绿复验，61.791s |
| `messagev3-1788417330333-18171` | pre | 83 | 55 | 11 | 4 | 13 | 修复部署前/旧镜像运行态，不能作为最终绿证 |

最终两轮不存在非 PASS；以下仅保留旧 suite 的历史失败摘要，不能覆盖当前结论：

- `TEST_FAILED`：Phase1 post 的 `G01A/G01B/G01F/G01H/G01K/G02A/G05A..G05E`；
  这是当轮 Cargo test 未通过或未执行，不应被转换成业务成功。
- `NOT_VERIFIED`：`G01I/G01J` 缺 `sqlite_readback` 关联，`G01L` 未达到可证明终态，
  `G02C` 的失败行/重试链未完成全部终态断言。
- `BLOCKED_PRECONDITION`：`G03A/G03B/G04E`、媒体 `G01C/G01D/G01E/G01G/G03C`、
  Phase4 `G05F/G05G/G14A/G14B/G14C` 的真实前置、投影或读取链未完成。

后续单 Gate 目录 `messagev3-1788418711287-79032` 和
`messagev3-1788423737009-77843` 仅重跑了 `Phase1/post/MV3-G01A`，均为 PRE
旧运行态下的 `TEST_FAILED`。不同 suite 不能相加，也不能拿单 Gate 结果覆盖全量账。

历史 92 Gate 结果只用于解释迁移前分母；不得把它与当前 83 Gate 的 summary 合并。
客户端 92 Gate 合同、Helix Spec 047 的 30 producer Gate 和 Native 83 Gate 是三个不同视角。

### 6.2 证据等级

| 等级 | 能证明什么 | 不能替代什么 |
| --- | --- | --- |
| FOCUSED | Helix command/state/Effect/失败路径的局部确定性行为 | 真实 Go/WS/SQLite/部署 |
| LOCAL_INTEGRATION | 本机 profile 下的 driver/端口/SQLite 链 | PRE Pod、远端镜像和真实凭据 |
| REAL_NATIVE_CHAIN | 指定 suite run 的真实 HTTP/WS、projection、durable read-back、EventSink、清理 | Angular DOM/UI |
| UI/DOM | 客户端生命周期、渲染和交互 | Native real-chain 的后端/持久化证据 |

每个 Gate 的 `evidence.json` 必须能关联同一 `suite_run_id`、Phase、domain、Gate 和
`corr_key`，并满足该 Gate 的 `http_business`、`projection`、`cleanup`，按适用性补齐
`ws`、`sqlite_readback`、`event_sink`、`persist`。`corr_key=unbound`、跨 run 借证或只给 health
均会被 runner 判为 `NOT_VERIFIED`/非 PASS。

## 7. 已落地修复与尚未闭环项

### 7.1 Helix 当前分支

- `481ab764`：加入 real-chain 环境 profile 和 CI 唯一入口；默认 profile 为 `pre`。
- `f3c3bca0`：兼容客户端账号池格式并记录远端复验边界。
- `b361b05a`：收敛真实发送资源清理与频道回收，确保 engine、WS、SQLite 和远端频道
  在发送 Gate 结束后按 run-owned 语义释放。

### 7.2 Go 侧外部提交（不属于本 Helix worktree）

- `d8db78f`：发送快照使用服务接收时间下限、同频道 snapshot→commit→primary post lane，
  关闭/删除频道缺失 active snapshot 时发送 terminal error ACK/drop，开放频道仍保留 NACK。
- Go 仓库已将相关变更合并到 `dev-paas2.5` / `pre-paas2.5` 的提交链；这里仅记录依赖，
  不把远端 Git commit 当成 PRE Pod 已部署证据。

### 7.3 已关闭的阻塞与剩余边界

本轮 83 Gate 中 `BLOCKED_PRECONDITION=0`。此前的迟到 WS、active member snapshot、上传、
搜索、read bits、首发终态与 Phase5 cleanup 阻塞均已通过最终 PRE suite 关闭。

这不等于 Angular UI/DOM 或未来 `dev` profile 已验收；两者仍属于独立证据边界。后续若
PRE 镜像、账号池、Java OSS 或搜索权限发生变化，应创建新 `suite_run_id` 重新验证，不能复用
本次 83/83 的旧证据。

## 8. 环境 profile、CI 与本地命令

### 8.1 已登记 profile

| profile | Go HTTP | Go WS | Java API | 说明 |
| --- | --- | --- | --- | --- |
| `pre`（默认） | `http://api-im-pre.jinqidongli.com/api/cses` | `ws://ws-im-pre.jinqidongli.com/api/v4/websocket` | `https://api-pre.jinqidongli.com` | PRE 专属远端链 |
| `local` | `http://127.0.0.1:18066/api/cses` | `ws://127.0.0.1:18066/api/v4/websocket` | `http://127.0.0.1:3399` | 本地服务链，必须显式切换 |
| `dev` | 未登记 | 未登记 | 未登记 | 未来扩展，不能默认猜测端点 |

health 默认由 Go HTTP base 派生为 `${goHttpBase}/health`；也可设置
`MESSAGEV3_GO_HEALTH_URL` 单独覆盖。

```bash
# 查看当前 83 个可执行 Gate
node scripts/messagev3/phase1-5-real-chain.mjs --list

# 默认 PRE，全量 83 Gate；任一非 PASS 会以非零退出
node scripts/messagev3/phase1-5-real-chain.mjs --all

# 本地 profile
MESSAGEV3_ENV=local node scripts/messagev3/phase1-5-real-chain.mjs --all

# 显式选择 profile
node scripts/messagev3/phase1-5-real-chain.mjs --env pre --all

# 临时自定义环境；三个端点必须同时给出
MESSAGEV3_ENV=dev \
MESSAGEV3_GO_HTTP_BASE=http://go-dev.example/api/cses \
MESSAGEV3_GO_WS_URL=ws://ws-dev.example/api/v4/websocket \
MESSAGEV3_JAVA_API_BASE=https://java-dev.example \
node scripts/messagev3/phase1-5-real-chain.mjs --all
```

单个端点覆盖变量为 `MESSAGEV3_GO_HTTP_BASE`、`MESSAGEV3_GO_WS_URL`、
`MESSAGEV3_JAVA_API_BASE`（兼容 `HELIX_MAIN_API_BASE`）。未知 profile 只有在三个端点
同时提供时才允许解析。

GitLab CI 的业务入口是：

```bash
node scripts/messagev3/phase1-5-real-chain.mjs --all
```

CI 使用 `MESSAGEV3_ENV=pre`，需要 GitLab File 变量 `CSES_WDIO_CREDENTIALS_FILE`；
结果上传到 `artifacts/messagev3-phase1-5-real-chain/`。`scripts/gate.sh` 仅保留给人工完整 Rust/FFI 检查，不与 real-chain 争用 CI 主入口。

## 9. PRE 重跑步骤与停止条件

1. **部署态确认**：记录 PRE `cses-im-server` Pod 的 image digest、启动时间和 readiness，
   不只依赖 Deployment 文本或 Git commit。
2. **运行态确认**：检查 WS/HTTP readiness；发现重复 NACK、`channel not found...` 或账号/
   Java OSS/搜索前置失效时，保留日志并按真实状态停止升级结论。
3. **同 profile 重跑**：在 Helix 当前分支执行 `node ... --all --env pre`，保留全新
   `suite_run_id`；不要复用旧 `evidence.json`。
4. **逐 Gate 对账**：先看 summary，再按同一 run 校验 HTTP business、Go WS、Helix projection、
   SQLite durable read-back、EventSink 和 cleanup；所有字段齐全才允许 PASS。
5. **异常取证**：若再出现迟到 WS 或 active snapshot 错误，同时保存对应 Pod 日志、Gate
   `evidence.json` 和 correlation；不手工补 terminal event、不 truncate 共享数据库。
6. **清理复核**：确认 actor/observer WS、engine、临时 SQLite/WAL/SHM 和 run-owned 频道均已
   释放；清理失败要保留为非 PASS，而不是为了绿色删除证据。

## 10. 运行证据、清理与安全

证据布局固定为：

```text
artifacts/messagev3-phase1-5-real-chain/<suite-run-id>/summary.json
artifacts/messagev3-phase1-5-real-chain/<suite-run-id>/<Phase>/<canonical-domain>/<GATE>/evidence.json
```

清理只允许作用于本轮 run-owned 资源：

- Native engine 和 actor/observer WS；
- 本轮 SQLite 文件及 WAL/SHM；
- 本轮创建的远端频道 fixture；
- 本轮注册的 pending/correlation/timer。

禁止 truncate 共享数据库、删除其他 suite 的 artifacts、把旧 run 证据复制到新 run，
或以手工发送消息补齐缺失的业务事件。

## 11. 代码与文档真源索引

| 真源 | 用途 |
| --- | --- |
| [`scripts/messagev3/phase1-5-real-chain.mjs`](scripts/messagev3/phase1-5-real-chain.mjs) | 83 分母、9 个排除项、证据校验、suite 编排 |
| [`scripts/messagev3/real-chain-environment.mjs`](scripts/messagev3/real-chain-environment.mjs) | `pre`/`local` profile、默认环境和端点覆盖 |
| [`crates/helix-driver-native/tests/messageV3-domain/README.md`](crates/helix-driver-native/tests/messageV3-domain/README.md) | Native 83 Gate 目录合同、运行方式、证据布局 |
| [`crates/helix-driver-native/tests/messageV3-domain/shared/mod.rs`](crates/helix-driver-native/tests/messageV3-domain/shared/mod.rs) | session、频道/成员 fixture、发送、WS、SQLite、清理 harness |
| [`crates/helix-driver-native/tests/messageV3-domain/handoff.md`](crates/helix-driver-native/tests/messageV3-domain/handoff.md) | Phase5 文字接龙与 domain 细节交接 |
| [`docs/harness/OPERATING.md`](docs/harness/OPERATING.md) | CI 与人工运行边界 |
| [`docs/GOAL.md`](docs/GOAL.md) | runtime-agnostic、确定性、MessageV3 Event/PersistOk 不变量 |
| [`docs/ROADMAP.md`](docs/ROADMAP.md) | 当前 Phase/Spec 方向和活账 |
| [`specs/047-im-event-gate-inventory/contracts/gate-migration.v1.json`](specs/047-im-event-gate-inventory/contracts/gate-migration.v1.json) | Spec 047 的 producer Gate 迁移合同 |
| [`specs/messagev3-phase1-4-integration-handoff.md`](specs/messagev3-phase1-4-integration-handoff.md) | 历史 Phase1–4 集成交接；仅作历史参考 |

## 12. 本轮完成清单

- [x] 确认 Helix 分支和 HEAD，没有覆盖用户现有未跟踪 artifacts。
- [x] 确认 `MESSAGEV3_ENV=pre` 和三端点解析结果。
- [x] 确认 83 个 Gate 和 9 个排除项未漂移。
- [x] 以最终源码执行全量 `--all --env pre` 并取得新 `suite_run_id`。
- [x] 83 个 Gate 的 HTTP/WS、projection、Persist/SQLite、EventSink、cleanup 证据通过 runner 合同。
- [x] `TEST_FAILED=0`、`NOT_VERIFIED=0`、`BLOCKED_PRECONDITION=0`。
- [x] `cargo test -p helix-im` 全包通过，交接状态更新为 `GREEN`。
