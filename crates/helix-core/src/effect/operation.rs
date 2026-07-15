use bytes::Bytes;

use super::{
    Correlation, DomainEventBytes, FileUploadRequest, HttpRequest, StorageOp, TimerId, TransportId,
};

/// 模块吐出的 I/O 意图声明（tagged union，非法组合不可表达）。
///
/// **执行顺序契约（EFFECT-5）**：同一个 step 产出的 `Vec<Effect>` 之间**无兑现顺序保证**——
/// driver 可乱序/并发兑现（如 Persist 入队即返回，Send/Emit inline await）。需要顺序的因果
/// **必须经 `Correlation` 串联**（吐 Request/Persist{corr} → 等 `PortReply` → 再吐下一步），
/// 不可依赖 `Vec` 下标顺序。
#[derive(Debug)]
pub enum Effect {
    /// 发送 WS 帧（driver 调用 FrameSender::send）
    Send {
        transport: TransportId,
        frame: Bytes,
    },

    /// 需成功回报的持久化：带 Correlation。
    /// driver 执行后**必须**产出 `Tick::PortReply{corr, outcome}`。
    /// 典型用途：batch_upsert 消息（写成功才推 cursor）。
    Persist {
        corr: Correlation,
        ops: Vec<StorageOp>,
    },

    /// 幂等 fire-and-forget 持久化：不带 Correlation，不产出 PortReply。
    ///
    /// ## 约束（强制）
    ///
    /// 仅用于满足以下**全部**条件的写操作：
    /// 1. 幂等：重复执行结果相同（如 monotonic_upsert 的 MAX guard 天然幂等）
    /// 2. 丢失可接受：写入失败后，下次操作可从旧状态重新推导正确结果
    ///
    /// **禁止**用于：
    /// - 首次落库的业务数据（消息、用户数据等）
    /// - 任何依赖"写入成功"才能继续的操作
    ///
    /// 典型用途：在 `Persist{corr}` 的 PortReply Ok 后推进单调水位（如同步 cursor）。
    PersistFire { ops: Vec<StorageOp> },

    /// HTTP 请求（**必达**语义）：带 Correlation。
    /// driver 执行后**必须**产出 `Tick::PortReply{corr, outcome=Ok(resp)/Err}`。
    ///
    /// ## 背压契约（与 `Persist` 对称，刀1 翻默认）
    ///
    /// driver 对必达 `Http` 在超载时**只能 block-back**（队满 `.await` 把背压传导回
    /// tick_tx，慢而不丢），**绝不 drop**——因为 PortReply 是 load-bearing：响应数据 /
    /// inflight-gate 清除 / cursor 推进都依赖它。
    /// 「满即丢本条」降级为显式 opt-in（见 `HttpFire`），不再是 Http 的全局默认。
    Http { corr: Correlation, req: HttpRequest },

    /// 本地文件上传（**必达**语义）：带 Correlation。
    /// driver 读取 `local_path` 指向的本地文件并执行真实上传后，**必须**产出
    /// `Tick::PortReply{corr, outcome=Ok(resp)/Err}`。
    UploadFile {
        corr: Correlation,
        req: FileUploadRequest,
    },

    /// 幂等 fire-and-forget HTTP（**可丢弃·丢失可自愈**）：不带 Correlation，不产出 PortReply。
    ///
    /// 与 `PersistFire` 对称——driver 对 `HttpFire` 用 DropNewest 溢出策略
    /// （队满丢本条/最新来的 + warn），泵不阻塞。
    ///
    /// ## 约束（强制，与 `PersistFire` 同口径）
    ///
    /// 仅用于满足以下**全部**条件的请求：
    /// 1. 不需要响应体（HTTP 仅触发 / ack，真数据走其它通道如 WS）
    /// 2. 丢失可自愈：丢了由后续 cursor-gate / 重连 / proactive resync 重发补偿
    /// 3. 无 inflight 闸依赖其 PortReply 清除
    ///
    /// **禁止**用于：
    /// - posts/create（发消息必达）
    /// - 任何 PortReply 推进状态机的请求（sync/notify、increment 等——它们的 PortReply
    ///   承载数据 / 清闸 / 推 cursor，丢了会自锁；这类必须用 `Http`）
    ///
    /// 当前 helix-im 无真 fire-and-forget HTTP（全部必达），HttpFire 暂为空桶，
    /// 为未来正确的请求类（如 telemetry ping / presence 心跳 HTTP）预留正确归属，
    /// 并让「Http 默认必达 Block」在类型层成立（opt-in 才放松）。
    HttpFire { req: HttpRequest },

    /// 主动请求宿主（EventBus REP 模式·方向① core→host→core）：带 Correlation。
    ///
    /// 与 `Http`/`Persist` 同构——driver 把请求转交宿主（UI / FFI host / JS），
    /// 宿主处理后**必须**产出 `Tick::PortReply{corr, outcome}`，
    /// 经 `ExecutionShell::corr_map` 定向回投给发起该 corr 的模块。
    ///
    /// - `kind`：编译期常量路由键（如 "ask_user" / "host_config"），core 不解析其含义
    /// - `payload`：已序列化的请求字节（由模块 ACL 序列化）
    ///
    /// ## 与 `Emit` 的区别
    ///
    /// `Emit` 是单向 PUB（fire-and-forget，无回音）；
    /// `Request` 是双向 REP 的**发起端**（要回音，靠 corr 配对，复用 PortReply 回路）。
    ///
    /// **driver 实现契约（EFFECT-2）**：driver 必须兑现并回灌 `Tick::PortReply{corr}`；若是
    /// deferred stub（如 native engine_loop 当前只 warn 不回灌），该 corr 会永久滞留 corr_map、
    /// inflight 闸不清——接线缺口在「第二个发起 Request 的模块接入」时暴露（与 Http/Persist 同口径）。
    Request {
        corr: Correlation,
        kind: &'static str,
        payload: Bytes,
    },

    /// 发布领域事件（同步 push，不等待）。
    /// driver 调用 EventSink::emit，事件字节已在 ACL-1 序列化。
    Emit { event: DomainEventBytes },

    /// 调度定时器：`after_ms` 毫秒后产出 `Tick::Timer{id}`。
    /// driver 维护 TimerRegistry；id 由模块分配（建议模块维护 IdSource）。
    ScheduleTimer { id: TimerId, after_ms: u64 },

    /// 取消定时器（幂等，id 不存在时静默忽略）。
    CancelTimer { id: TimerId },
}
