# MessageV3 Gate 测试局部指令

> 本文件约束 `crates/helix-im/tests/message_v3/` 全树。上级规则见
> [`../../AGENTS.md`](../../AGENTS.md) 与仓库根 `AGENTS.md`。

## 使命与边界

本目录是 Spec 047 当前 30 个 MessageV3 Gate 的 Rust 行为测试与 canonical JSON
生产入口。每个 Gate 都必须通过真实 `ImModule::handle` 回放得到 `Effect::Emit`
事件，并在需要生成证据时写出 sender/receiver 双视角产物。

这里证明的是 Helix producer 的 deterministic、contract 和 Rust driver 接缝行为。
它不能替代 PC WDIO、Mobile GTest、真实 Go/WS、双账号运行态或 consumer canonical
验收。未执行的消费端证据必须继续标为 `NOT_RUN` 或 `NOT_SYNCED_BY_SCOPE`。

## 必读真源

1. Gate 分母、测试 target、源码路径和迁移状态：
   [`../../../../specs/047-im-event-gate-inventory/contracts/gate-migration.v1.json`](../../../../specs/047-im-event-gate-inventory/contracts/gate-migration.v1.json)
2. MessageV3 `{event,data}` schema：
   [`../../../../specs/047-im-event-gate-inventory/contracts/message-v3-event.schema.json`](../../../../specs/047-im-event-gate-inventory/contracts/message-v3-event.schema.json)
3. 四份产物 schema：
   [`../../../../specs/047-im-event-gate-inventory/contracts/gate-inbound.schema.json`](../../../../specs/047-im-event-gate-inventory/contracts/gate-inbound.schema.json)、
   [`../../../../specs/047-im-event-gate-inventory/contracts/gate-artifact.schema.json`](../../../../specs/047-im-event-gate-inventory/contracts/gate-artifact.schema.json)、
   [`../../../../specs/047-im-event-gate-inventory/contracts/gate-provenance.schema.json`](../../../../specs/047-im-event-gate-inventory/contracts/gate-provenance.schema.json)
4. 执行与人工验收手册：[`README.md`](README.md)

## 目录与所有权

- `post/`：消息发送、重试、撤回、转发、reaction、加急、模板确认。
- `schedule/`：定时消息创建和取消。
- `timeline/`：latest/older/newer、append、anchor、定位、回复分支。
- `read/`：会话已读和单条消息回执。
- `channel/`：建群、话题、关闭/退出、群属性更新。
- `channel_member/`：成员 roster 与离群广播。
- `sync/`：hello、离线 sync、pong gap、陌生频道补全。
- `contracts/`：事件信封、产物 schema、30 Gate 注册表合同。
- `support/`：Scenario 回放和 canonical JSON writer，只提供通用机制，不承载
  某个 Gate 的业务字段。

每个 Gate 是独立 Cargo integration test target。新增、删除或改名必须同时更新
`crates/helix-im/Cargo.toml` 和 migration manifest；缺失、额外或重复 Gate 都必须失败。

## 产物生成合同

- 普通 `cargo test -p helix-im --test <target>` 只验证行为，不写 JSON。
- 只有设置 `HELIX_GATE_ARTIFACT_DIR` 时，Gate 测试才写出四份 JSON。
- `HELIX_HEAD` 必须使用当前 40 位 commit SHA，进入 `provenance.json`。
- Rust 测试负责构造全部业务 `ticks/event/data/storage_basis`。
- `generate-v1-evidence.mjs` 只编排 Cargo target、检查四文件集合、运行 schema
  校验并计算 hash；禁止在 JavaScript 中拼装业务 payload。
- `.v1` 是被 Git 忽略的本地证据目录，不得成为生产代码依赖，也不得解释成旧 V1
  运行时协议。

每个 Gate 固定生成：

```text
.v1/gates/<Gate>/generated/
├── inbound.json
├── outbound.sender.json
├── outbound.receiver.json
└── provenance.json
```

## 双视角不变量

- sender 与 receiver 文件必须始终同时存在。
- 每个业务事件只能包含 `{event,data}`，禁止 Frame、Surface、capabilities、别名字段
  或要求壳自行计算的 delta。
- 双端传播 Gate 必须对账到同一 server identity；viewer-local 未读、角色、已读、
  anchor 和导航状态必须独立计算。
- persona-local Gate 的 receiver `events: []` 是明确的“不传播”证据，不得复制 sender
  事件制造假双视角。
- receiver 错过在线事件后的恢复属于独立 Gate，不得拿 sender 快照替代 receiver 回放。
- terminal Emit 必须发生在本地持久提交之后，HTTP admission 或 command accepted
  不能冒充完成。

## 修改流程

1. 从 migration manifest 确认 Gate ID、domain、target 和现有源码路径。
2. 先补或修改 Gate 内的行为断言，再修改生产代码。
3. 用不带产物环境变量的单 Gate 命令验证行为。
4. 用 `HELIX_GATE_ARTIFACT_DIR` 重新生成该 Gate 的四份 JSON。
5. 人工比较 sender/receiver identity、事件可见性和 viewer-local 字段。
6. 执行 artifact schema、Gate registry 和 cutover checker。
7. 影响共享行为时再执行 30 Gate 总生成与 `cargo test -p helix-im --quiet`。

## 禁止事项

- 禁止手改 `generated/*.json` 修测试。
- 禁止让 JS、README fixture 或 consumer reducer成为业务 payload producer。
- 禁止在 sender/receiver 间复制未读数、权限、角色或导航状态。
- 禁止恢复 InteractionFrame、Surface、Projector、V1 fallback 或双写路径。
- 禁止把 `.v1/gates/<Gate>/inbound-sample.json`、
  `outbound-sample.json` 人工基线倒灌为 generated 权威产物。
- 禁止用单 Gate PASS 宣称 PC/Mobile/真实双用户 Green。

## 最小验收

```bash
# 单 Gate 行为，不生成 JSON
cargo test -p helix-im --test message_v3_g01 -- --nocapture

# 全部 30 Gate 运行并生成 120 个 JSON
node specs/047-im-event-gate-inventory/scripts/generate-v1-evidence.mjs

# schema、集合、旧链零引用与负例自测
node specs/047-im-event-gate-inventory/scripts/check-message-v3-cutover.mjs \
  --require-generated --self-test
```
