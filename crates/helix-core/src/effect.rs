//! # effect.rs
//!
//! `Effect` 是模块吐出的"想做的 I/O"声明，core 不执行，由 driver 兑现。
//!
//! ## 设计要点
//!
//! - `Persist` 强制带 `Correlation`（类型层逼模块走 peek/commit 两段式，轴⑤）
//! - `PersistFire`：fire-and-forget，**仅用于幂等且丢失可接受的写操作**
//!   （如 monotonic_upsert 在 Persist{corr} 成功后推进单调水位）
//!   禁止用于非幂等业务数据写入（如初次落库消息）
//! - `EffectSink`：跨 step 复用，clear 而非重建，零分配（轴①）
//!   原草稿 EffectBuffer/EffectBatch/EffectSink 三名字收敛为一个（审查意见2 S4）

use bytes::Bytes;

/// 不透明令牌：把 `Effect::Persist` / `Effect::Http` 与未来的 `Tick::PortReply` 配对。
///
/// `ExecutionShell` 维护 `Correlation → module_index` 映射（`corr_map`），
/// 保证 PortReply 定向投递给发出该 Correlation 的模块（审查意见2 P3 落实）。
///
/// ## MIN-1 修复
///
/// 内部值改为 `pub(crate)` — 外部 crate 通过 `raw()` 只读访问，
/// 只有 engine 的 `CorrelationSource` 可以创建新值，保证"ID 由分配器单调发放"不变量。
/// helix-im 通过 `ImModule::alloc_corr()` 获取（`helix_core::Correlation::new_from_raw`
/// 是 pub(crate) 构造器，只对 helix-core 自身可见）。
///
/// 由于 helix-im 是独立 crate，需要分配 Correlation，保留一个 `pub` 的 `from_raw`
/// 构造器（但语义上调用方应通过模块内的 IdAllocator 管理，不得随意构造）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Correlation(pub(crate) u64);

impl Correlation {
    /// 从原始 u64 构造（供 helix-im 等业务 crate 的 IdAllocator 使用）。
    ///
    /// 调用方负责保证全局单调唯一性；不得在 ID 分配器之外随意调用。
    pub fn from_raw(v: u64) -> Self {
        Self(v)
    }

    /// 获取内部原始值（只读）
    pub fn raw(self) -> u64 {
        self.0
    }
}

/// 不透明定时器 ID：把 `Effect::ScheduleTimer` 与未来的 `Tick::Timer` 配对。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TimerId(pub(crate) u64);

impl TimerId {
    /// 从原始 u64 构造（供业务 crate 的 IdAllocator 使用）
    pub fn from_raw(v: u64) -> Self {
        Self(v)
    }

    /// 获取内部原始值（只读）
    pub fn raw(self) -> u64 {
        self.0
    }
}

/// 传输通道 ID（模块可管理多个 transport，如 WS + debug transport）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TransportId(pub(crate) u32);

impl TransportId {
    pub fn from_raw(v: u32) -> Self {
        Self(v)
    }

    pub fn raw(self) -> u32 {
        self.0
    }
}

/// 零拷贝事件字节。`bytes::Bytes` 引用计数，clone = O(1)。
/// core 不解析内容，只搬运已序列化的字节。
#[derive(Debug, Clone)]
pub struct DomainEventBytes(pub Bytes);

/// 存储操作（具体 SQL 由 driver 的 Storage 实现构造）
#[derive(Debug, Clone)]
pub enum StorageOp {
    /// batch upsert：INSERT ... ON CONFLICT(conflict_key) DO UPDATE ...
    BatchUpsert(UpsertSpec),
    /// 通用单调 MAX-guard 写：仅当 new value > 当前值才写（CAS / 单调寄存器语义，严格 >）。
    MonotonicUpsert(MonotonicUpsertSpec),
    /// batch update：UPDATE table SET patch WHERE key_col IN (key_vals)
    BatchUpdate(BatchUpdateSpec),
    /// get：SELECT * FROM table WHERE ...（单行）
    Get(GetSpec),
    /// scan：SELECT * FROM table [LIMIT n]（多行，C3）
    Scan(ScanSpec),
}

/// BatchUpsert 规格
///
/// ## MAJ-5 修复
///
/// `table` 和 `conflict_key` 改为 `&'static str`，消除热路径的运行时 String 分配。
/// 表名和冲突键都是编译期已知的常量（"messages"、"temporary_id"），
/// `"messages".to_string()` 每次消息落库都产生一次堆分配，改为静态引用零成本。
///
/// 注意：`conflict_key` 的具体值（如 `"temporary_id"`）由 ACL-1 在 helix-im 侧提供，
/// helix-core 只传递字符串，不理解其含义。
#[derive(Debug, Clone)]
pub struct UpsertSpec {
    /// 目标表名（编译期常量，如 "messages"）
    pub table: &'static str,
    pub rows: Vec<Row>,
    /// ON CONFLICT 的列名（如 ACL-1 提供 "temporary_id"；core 不知道含义）
    pub conflict_key: Option<&'static str>,
}

/// 通用单调 MAX-guard 写规格：仅当 `value > 当前值` 才写入（CAS / 单调寄存器语义）。
///
/// 通用原语，core 不含任何业务含义——典型用途：增量同步水位、presence 心跳时间戳、单调计数器。
/// `scope_key` 是运行时值（由上层提供，如 channel_id.to_string()），core 不解析其含义。
#[derive(Debug, Clone)]
pub struct MonotonicUpsertSpec {
    pub scope_key: String,
    pub value: i64,
}

/// batch_update 规格
///
/// MAJ-5：`table` 和 `key_col` 改为 `&'static str`（编译期常量）
#[derive(Debug, Clone)]
pub struct BatchUpdateSpec {
    pub table: &'static str,
    pub key_col: &'static str,
    pub key_vals: Vec<SqlValue>,
    pub patch: Row,
}

/// get 规格
///
/// MAJ-5：`table` 和 `key_col` 改为 `&'static str`
#[derive(Debug, Clone)]
pub struct GetSpec {
    pub table: &'static str,
    pub key_col: &'static str,
    pub key_val: SqlValue,
}

/// scan 规格：通用多行扫描读（评审待办 C3，MVP 前置阻塞项）。
///
/// 典型用途：模块 on_start 时一次性载入全部单调水位（如 helix-im 扫
/// channel_event_cursor 全表做 proactive resync）。通用读，core 不含业务语义。
/// `GetSpec` 保持单行语义不变，多行走本类型。
#[derive(Debug, Clone)]
pub struct ScanSpec {
    /// 目标表名（编译期常量）
    pub table: &'static str,
    /// 最多返回行数（None = 全表；driver 应设硬上限防御）
    pub limit: Option<u32>,
}

/// SQL 值类型（不含数据库具体类型）
#[derive(Debug, Clone)]
pub enum SqlValue {
    Text(String),
    Integer(i64),
    Real(f64),
    Null,
}

/// 行数据（列名 → SQL 值）
pub type Row = Vec<(String, SqlValue)>;

/// HTTP 请求（driver 的 HttpRequester 解析并发出）
#[derive(Debug, Clone)]
pub struct HttpRequest {
    pub method: String, // "GET" / "POST" / etc.
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<Bytes>,
}

/// HTTP 响应（driver 包装后通过 PortReply 回传）
#[derive(Debug, Clone)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Bytes,
}

/// 模块吐出的 I/O 意图声明（tagged union，非法组合不可表达）
#[derive(Debug)]
pub enum Effect {
    /// 发送 WS 帧（driver 调用 Transport::send）
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

    /// HTTP 请求（带 Correlation）。
    /// driver 执行后产出 `Tick::PortReply{corr, outcome=Ok(resp)/Err}`。
    Http { corr: Correlation, req: HttpRequest },

    /// 发布领域事件（同步 push，不等待）。
    /// driver 调用 EventSink::emit，事件字节已在 ACL-1 序列化。
    Emit { event: DomainEventBytes },

    /// 调度定时器：`after_ms` 毫秒后产出 `Tick::Timer{id}`。
    /// driver 维护 TimerRegistry；id 由模块分配（建议模块维护 IdSource）。
    ScheduleTimer { id: TimerId, after_ms: u64 },

    /// 取消定时器（幂等，id 不存在时静默忽略）。
    CancelTimer { id: TimerId },
}

// ─── EffectSink ──────────────────────────────────────────────────────────────

/// Effect 输出缓冲（写入端）。
///
/// 由 `ExecutionShell` 持有，跨 `step()` 调用复用（`clear` 而非 `drop+new`），
/// 实现热路径零分配（轴①）。
///
/// 模块通过 `&mut EffectSink` 参数写入 effect，driver 通过 `as_slice()` 读取。
/// 原草稿中 `EffectBuffer` / `EffectBatch<'_>` / `EffectSink` 三个名字统一为此类型
/// （审查意见2 S4 落实）。
pub struct EffectSink {
    buf: Vec<Effect>,
}

impl EffectSink {
    pub fn new() -> Self {
        Self {
            buf: Vec::with_capacity(8),
        }
    }

    /// 写入一个 Effect
    #[inline]
    pub fn push(&mut self, e: Effect) {
        self.buf.push(e);
    }

    /// 清空缓冲（跨 step 复用，不释放内存）
    #[inline]
    pub fn clear(&mut self) {
        self.buf.clear();
    }

    /// 读取本次 step 产出的全部 Effect
    #[inline]
    pub fn as_slice(&self) -> &[Effect] {
        &self.buf
    }

    /// 是否有 Effect 产出
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }
}

impl Default for EffectSink {
    fn default() -> Self {
        Self::new()
    }
}
