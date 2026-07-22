# G14/G14a Implementation Handoff

## 状态

`PARTIAL - received ready; confirmation remains GAP`

## 已完成

- `ParticipantReceiptsProjector` 对 `TemplateConfirmation` 的非 forbidden 数据 fail-closed；类型存在不再允许伪造 ready confirmation。
- 新增 read-receipt UI：ready/loading/failed/forbidden/empty，received 与 confirmation blocked 明确分开；forbidden 不泄露成员计数。
- `im_template_received` canonical `{postId}` wire 与 GAP 边界有 focused contract。

## 证据

- Angular：`evidence/angular-focused.log`，2/2 PASS。
- Rust：focused final 尚待 root 在 shared 编译修复后聚合重跑。

## 未决

- shared `markRead/templateReceived` binding、authority/PersistOk 与 timeline/thread terminal Frame。
- `template.confirm` 继续 GAP，不得进入 017 Green 计数。
