# Quickstart: G15a PC 创建群聊验收

## 1. Artifact Gates

```bash
python3 .specify/extensions/helix-loop/scripts/loop_engine.py status
python3 .specify/extensions/helix-loop/scripts/loop_engine.py verify --gate R3
```

期望：R1、R2、R3 为 pass，`next_action` 指向 implementation；任何 fingerprint 变化先回到最早失效 gate。

## 2. Focused Rust Evidence

```bash
cargo test -p helix-im --test interaction_frame_contract_test
cargo test -p helix-im --test action_dispatch_test
cargo test -p helix-im --test channel_create_replay_test
```

必须证明：根级 binding 可调用；非法/stale 输入零 HTTP Effect；权威事实先产生单个 `PersistAtomic`；没有 `PersistOk` 就没有成功 Frame；重复/乱序/恢复只产生唯一结果。

## 3. Focused PC Evidence

在 `/System/Volumes/Data/workspace/angular/cses-client`：

```bash
yarn test --runInBand --runTestsByPath \
  src/pages/message-v4/surface-runtime/message-surface-runtime-view.component.spec.ts \
  src/pages/message-v4/components/message-ui-create-chat.component.spec.ts
```

必须证明：typed binding 打开 overlay；合法草稿携带原 binding 提交；失败保留草稿；只有 Frame insertion 才显示新群聊；不调用旧 direct create path。

## 4. Cross-Contract Evidence

```bash
cargo test -p helix-im --test projection_schema_test
rg -n "channel\.create|DialogListVm" crates/helix-im/contracts /System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4
```

Rust、生成 wire 合同、TypeScript 与 fixture 必须同批同步，不允许只改一侧。

## 5. Real-Chain Acceptance

使用创建者、受邀者、未参与者三个隔离账号，按项目现有真 Go harness 执行：正常创建、重复投递、断线恢复、远端拒绝与本地持久化失败注入。数据库检查仅只读，记录 request id、channel id、两用户独立 Frame correlation、未参与用户零变化。

每条命令在 `evidence/` 记录开始时间、结束时间、wall-clock、退出码、重试/跳过/阻塞原因和 SHA256。缺少真实环境时结论为 blocked，不用 fixture 代替真链。

## 6. Scope Guard

```bash
rg -n "mobile|flutter|ffi" specs/003-g15a-create-channel-pc/tasks.md
```

除明确的“不得新增移动端任务”验证文字外，任务不得修改移动端/Flutter/FFI 路径。
