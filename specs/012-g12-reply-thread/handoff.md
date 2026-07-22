# G12 Handoff

## 状态

`PARTIAL - WAITING_SHARED_PERSIST_AND_G01_REPLY_ADAPTER`

## 已完成

- reply projection 在 finalize 时校验 request channel/root；mixed-channel 或 unrelated root authority 整批清空，不泄漏到 renderer。
- 保留 snapshot/append revision 与跨页去重既有逻辑。
- 新增 `g12_reply_thread_interaction_test.rs`。
- 既有 reply drawer focused suite 覆盖 root/child target、分页呈现、Enter/Shift+Enter、receipt 后清 draft 与旧 receipt 保留 draft，`5/5 PASS`。

## 证据

- Angular schedule/runtime/reply 合并 batch `16/16 PASS`，wall-clock `25.33s`。
- Rust aggregate 待 root 统一执行。

## 待 root 收敛

- query graph 的 HTTP data 必须先 PersistAtomic(messages/edges/page coverage) 再发布 ThreadDetail Frame。
- `replyMessage` 只能 adapter 到 G01 replied send authority，禁止复制发送状态机。
- 最终 thread/timeline summary 同 Frame、双账号/双库/视觉证据归 017。
