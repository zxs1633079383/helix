# Contract: InteractionFrame V2-only

## Producer

1. 生产入口只能调用统一 V2 builder/emitter。
2. durable 业务 mutation 必须由 matching PersistOk 解锁。
3. 每个 mutation 包含 surface、scope、surfaceEpoch、baseRevision、revision 和完整 typed VM。
4. 同一 authority fact 影响的已 attach Surface 进入一个 transaction；未 attach Surface 不制造 mutation。
5. directive 只能描述提交后 focus/reveal/clear/detach，不携带业务 payload。

## Consumer

1. 生产 codec 只接受 `schemaVersion=2`。
2. 整帧先校验，再 staging，再一次 version swap；任一 mutation 非法则零提交。
3. slot 必须比较 scope、surfaceEpoch、baseRevision/revision。
4. transactionId 重放幂等；gap 进入 scoped resync，不调用 raw reducer。
5. FrameApplied 只在整帧提交和 directive 调度完成后发送，包含 transactionId、applied slots 和结果。

## Negative Contract

- V1 Frame：拒绝并计数，零 normalize、零业务 VM 写入。
- unknown schema/content/surface：拒绝，保留上一版 VM。
- stale epoch/actionRef：拒绝，零 HTTP/Storage/Frame。
- PersistErr：零成功 Frame。
- directive failure：已提交 Frame 不回滚，但 ack 记录 directive 结果。
- duplicate authority/transaction：零新增业务 mutation。

## Gate Payload Ownership

Gate payload、Surface、VM、ActionBinding 和组件映射以 [strict-matrix.md](../strict-matrix.md) 与 [per-gate-spec-index.md](../per-gate-spec-index.md) 为准，不在本合同复制 25 行。
