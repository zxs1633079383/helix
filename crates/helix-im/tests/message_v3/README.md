# MessageV3 Gate 测试与 JSON 验收手册

## 1. 这套测试解决什么问题

本目录把 Spec 047 的 30 个 IM Gate 分别做成独立 Rust integration test target。
每个 Gate 使用确定性输入驱动真实 `ImModule::handle`，检查持久化前后顺序和
`Effect::Emit { event: DomainEventBytes }`（bytes 内含 `{event,data}`），并可从同一次回放写出 sender/receiver 双视角
canonical JSON。

完整链路是：

```text
Gate Rust test
  -> 构造 Tick / PortReply
  -> ImModule::handle
  -> Persist / PersistOk
  -> Effect::Emit { event: DomainEventBytes }
  -> sender/receiver 事件断言
  -> Rust artifact writer
  -> .v1/gates/<Gate>/generated/*.json
  -> schema + SHA-256 + cutover checker
```

JavaScript 生成器不是业务 producer。它只读取
[`gate-migration.v1.json`](../../../../specs/047-im-event-gate-inventory/contracts/gate-migration.v1.json)，
逐个执行 manifest 登记的 Cargo target，并负责文件集合、schema 和 hash 校验。

## 2. 30 个 Gate 与测试位置

| Gate | 功能 | Cargo target | 测试源码 |
|---|---|---|---|
| G-01 | 发送文本消息与频道绝对态 | `message_v3_g01` | `post/g01/main.rs` |
| G-02 | 发送文档消息 | `message_v3_g02` | `post/g02/main.rs` |
| G-03a | 失败消息重试 | `message_v3_g03a` | `post/g03a/main.rs` |
| G-04 | 在线撤回消息 | `message_v3_g04` | `post/g04/main.rs` |
| G-04a | 离线同步撤回 | `message_v3_g04a` | `post/g04a/main.rs` |
| G-05 | 转发与批量发送结果 | `message_v3_g05` | `post/g05/main.rs` |
| G-06 | reaction/quickReply 绝对态 | `message_v3_g06` | `post/g06/main.rs` |
| G-07 | 加急与确认 | `message_v3_g07` | `post/g07/main.rs` |
| G-08 | 创建定时消息 | `message_v3_g08` | `schedule/g08/main.rs` |
| G-09 | 取消定时消息 | `message_v3_g09` | `schedule/g09/main.rs` |
| G-10 | latest bounded timeline | `message_v3_g10` | `timeline/g10/main.rs` |
| G-10a | 加载 older 历史 | `message_v3_g10a` | `timeline/g10a/main.rs` |
| G-10b | 加载 newer 消息 | `message_v3_g10b` | `timeline/g10b/main.rs` |
| G-10c | 满窗口追加与淘汰 | `message_v3_g10c` | `timeline/g10c/main.rs` |
| G-10d | 离底 anchor 与新消息提示 | `message_v3_g10d` | `timeline/g10d/main.rs` |
| G-11 | 已缓存消息定位 | `message_v3_g11` | `timeline/g11/main.rs` |
| G-11a | 未缓存定位与双向分页 | `message_v3_g11a` | `timeline/g11a/main.rs` |
| G-12 | 回复列表与话题分支 | `message_v3_g12` | `timeline/g12/main.rs` |
| G-13 | 当前 viewer 会话已读 | `message_v3_g13` | `read/g13/main.rs` |
| G-14 | 单条消息已读回执 | `message_v3_g14` | `read/g14/main.rs` |
| G-14a | 模板消息“已收到” | `message_v3_g14a` | `post/g14a/main.rs` |
| G-15a | 创建群聊与角色差异 | `message_v3_g15a` | `channel/g15a/main.rs` |
| G-15 | 创建话题 | `message_v3_g15` | `channel/g15/main.rs` |
| G-16 | 退出或关闭群聊 | `message_v3_g16` | `channel/g16/main.rs` |
| G-16b | 成员退出与 roster 广播 | `message_v3_g16b` | `channel_member/g16b/main.rs` |
| G-16c | 修改群名称、简介或公告 | `message_v3_g16c` | `channel/g16c/main.rs` |
| G-S1 | hello 后增量初始化 | `message_v3_gs1` | `sync/gs1/main.rs` |
| G-S2 | 离线 sync notify | `message_v3_gs2` | `sync/gs2/main.rs` |
| G-S3 | pong gap 补偿 | `message_v3_gs3` | `sync/gs3/main.rs` |
| G-S4 | 陌生频道自动补全 | `message_v3_gs4` | `sync/gs4/main.rs` |

五个公共合同 target：

| 合同 | Cargo target | 职责 |
|---|---|---|
| MessageV3 信封 | `message_v3_event_contract` | 事件只能包含 `event` 与 `data` |
| 产物 schema | `message_v3_artifact_schema` | 校验四类 JSON、字段禁集和全树 |
| Gate 注册表 | `message_v3_gate_registry` | 校验 manifest、Cargo target 和 30 Gate 集合完全一致 |
| Phase1 registry | `message_v3_phase1_registry` | 校验 Phase1 Gate target 与 manifest 映射 |
| Phase3 registry | `message_v3_contracts_phase3_registry` | 校验 Phase3 contract target 与 manifest 映射 |

## 3. 执行单个 Gate，不生成 JSON

普通测试适合开发阶段快速验证行为：

```bash
cd "$(git rev-parse --show-toplevel)"

cargo test -p helix-im --test message_v3_g01 -- --nocapture
```

这个命令会执行 G-01 target 内全部测试，包括 sender/receiver 行为断言。因为没有
`HELIX_GATE_ARTIFACT_DIR`，产物测试在完成行为回放后不会写文件。

其他 Gate 只需替换 target：

```bash
cargo test -p helix-im --test message_v3_g03a -- --nocapture
cargo test -p helix-im --test message_v3_g16b -- --nocapture
cargo test -p helix-im --test message_v3_gs1 -- --nocapture
```

从 Gate ID 查询准确 target 和源码路径：

```bash
GATE=G-04

jq -r --arg gate "$GATE" \
  '.entries[] | select(.gate_id == $gate) |
   [.test_target,.test_path] | @tsv' \
  specs/047-im-event-gate-inventory/contracts/gate-migration.v1.json
```

## 4. 执行单个 Gate，并自动生成 JSON

下面以 G-01 为例：

```bash
cd "$(git rev-parse --show-toplevel)"

OUT="$PWD/specs/047-im-event-gate-inventory/.v1/gates/G-01/generated"
mkdir -p "$OUT"

HELIX_GATE_ARTIFACT_DIR="$OUT" \
HELIX_HEAD="$(git rev-parse HEAD)" \
cargo test -p helix-im --test message_v3_g01 -- --nocapture
```

测试通过后，Rust writer 自动写出：

```text
specs/047-im-event-gate-inventory/.v1/gates/G-01/generated/
├── inbound.json
├── outbound.sender.json
├── outbound.receiver.json
└── provenance.json
```

要生成其他 Gate，必须同时替换输出目录和 Cargo target，避免把一个 Gate 的产物写进
另一个 Gate 目录。

## 5. 四份 JSON 分别证明什么

### `inbound.json`

- `gate_id`：当前 Gate。
- `ticks`：真实测试喂给 `ImModule::handle` 的确定性输入序列。
- `remote_basis`：相关远端 PostgreSQL 表和真实列名依据。

### `outbound.sender.json`

- `perspective: sender`。
- sender 实际可见的 MessageV3 `{event,data}` 序列。
- sender 本地 SQLite `storage_basis`。

### `outbound.receiver.json`

- `perspective: receiver`。
- receiver 实际可见的事件序列。
- receiver 自己的本地 SQLite `storage_basis`。
- persona-local Gate 可以合法地得到 `events: []`，用于证明没有错误传播。

### `provenance.json`

- producer Gate、Cargo test target 和 `ImModule::handle` 入口。
- 当前 `helix_head`。
- storage schema hash。
- inbound/sender/receiver 三份文件的 SHA-256。
- `consumer_sync: NOT_SYNCED_BY_SCOPE`，明确当前没有冒充 PC/Mobile 已同步。

## 6. 一次执行全部 30 个 Gate

推荐使用总生成器：

```bash
cd "$(git rev-parse --show-toplevel)"

node specs/047-im-event-gate-inventory/scripts/generate-v1-evidence.mjs
```

它会串行执行 manifest 中的 30 个 Cargo target，为每个 target 设置
`HELIX_GATE_ARTIFACT_DIR` 和 `HELIX_HEAD`，检查每个 Gate 恰好四份文件，然后执行
`message_v3_artifact_schema`。

预期摘要：

```json
{
  "verdict": "PASS",
  "gates": 30,
  "files": 120,
  "consumer_sync": "NOT_SYNCED_BY_SCOPE"
}
```

检查实际数量：

```bash
find specs/047-im-event-gate-inventory/.v1/gates \
  -path '*/generated/*.json' -type f | wc -l
```

预期为 `120`。

如只想执行全部 Gate 行为而不生成 JSON：

```bash
cargo test -p helix-im --quiet
```

这还会运行 `helix-im` 的其他 unit/integration tests，不仅是 30 个 Gate。

## 7. 执行公共合同与物理硬切验收

```bash
cargo test -p helix-im --test message_v3_event_contract -- --nocapture
cargo test -p helix-im --test message_v3_gate_registry -- --nocapture

HELIX_GATE_ARTIFACT_ROOT="$PWD/specs/047-im-event-gate-inventory/.v1/gates" \
cargo test -p helix-im --test message_v3_artifact_schema -- --nocapture

node specs/047-im-event-gate-inventory/scripts/check-message-v3-cutover.mjs \
  --require-generated --self-test
```

最后一条必须得到 Gate missing/extra/duplicate 均为 0、generated 集合为 `EXACT`、
旧 Frame/Surface/V1 active 引用为 0。

## 8. 双视角人工验收

先查看同一 Gate 的两份结果：

```bash
GATE=G-01
BASE="specs/047-im-event-gate-inventory/.v1/gates/$GATE/generated"

jq . "$BASE/outbound.sender.json"
jq . "$BASE/outbound.receiver.json"
```

通用检查项：

1. 两份文件的 `gate_id` 相同，`perspective` 分别为 `sender`、`receiver`。
2. 每个事件只含 `event` 和 `data`。
3. 双端传播时，双方 server message/channel identity 一致。
4. `unreadCount`、已读、角色、权限、anchor 和导航状态按 viewer 独立计算。
5. sender-only 失败、重试中间态或私有定时消息不得出现在 receiver。
6. terminal 事件必须出现在相关 PersistOk 之后。
7. receiver `events: []` 必须能由 persona-local 语义解释，不能因为测试漏跑而为空。

关键 Gate 验收：

- **G-01**：双方 `im:post:received.data.id` 相同；sender
  `im:channel:update.unreadCount=0`；receiver 等于自己的既有绝对值加 1；双方
  `lastPost.id/userId/message/createAt` 相同。
- **G-03a**：sender 顺序为 failed、sending、received；三个阶段复用同一
  `temporaryId`；receiver 只在成功后看到 received，不出现 failed/sending。
- **G-04**：双方针对同一 post 得到 `im:post:revoke` 且 `revoke=true`；若该消息是
  lastPost，双方还得到各自绝对态的 `im:channel:update`；撤回不增加未读。
- **G-06**：`quickReply` 是唯一数组形状，不出现 `props.quickReply`、
  `reactions` 或 `reactionCount`。
- **G-08/G-09**：receiver `events: []`，证明创建/取消定时消息不传播。
- **G-13**：当前 viewer 清理自己的未读，receiver 不被远程清零。
- **G-16b**：remaining member 与 leaver 收到符合各自身份的 roster/lifecycle 事件。

可快速列出事件名：

```bash
jq -r '.events[].event' "$BASE/outbound.sender.json"
jq -r '.events[].event' "$BASE/outbound.receiver.json"
```

可比较双方收到的 server message identity：

```bash
jq -r '.events[] |
  select(.event == "im:post:received") | .data.id' \
  "$BASE/outbound.sender.json" "$BASE/outbound.receiver.json"
```

## 9. generated 与 sample 的区别

每个 Gate 根目录可能还保留：

```text
inbound-sample.json
outbound-sample.json
```

它们是规格阶段的冻结人工审核基线，不会被生成脚本重写。实现完成后，
`generated/inbound.json`、`generated/outbound.sender.json`、
`generated/outbound.receiver.json` 和 `generated/provenance.json` 才是 Rust 测试生成的
权威合同。禁止把 sample 手工复制到 generated。

## 10. Git 与证据边界

`specs/047-im-event-gate-inventory/.v1/` 被 `.gitignore` 忽略：

- 重新 checkout 后需要重新执行生成器。
- 生成 120 个 JSON 不应污染 `git status`。
- 本地 JSON PASS 只证明 Helix producer。
- PC WDIO、Mobile GTest、真实 Go/WS、真实双用户和 consumer byte-exact 同步仍需后续
  独立验收，不能借用本目录的 PASS。
