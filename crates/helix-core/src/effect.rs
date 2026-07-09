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
    /// 守卫式自增：`UPDATE … SET bump_col = bump_col + delta[, set…] WHERE key=? AND ? > guard_col`。
    /// 通用计数器原语（无 read-modify-write），见 `GuardedBumpSpec`。
    GuardedBump(GuardedBumpSpec),
    /// get：SELECT * FROM table WHERE ...（单行）
    Get(GetSpec),
    /// scan：SELECT * FROM table [LIMIT n]（多行，C3）
    Scan(ScanSpec),
    /// batch delete：`DELETE FROM table WHERE scope_col = ? AND key_col IN (key_vals)`。
    /// 复合 PK 作用域删除（如 channel_member 成员离场：scope=channel_id + key=user_id）。
    /// O(k) 单语句；scope 等值约束**绝不跨作用域误删**。见 `BatchDeleteSpec`。
    BatchDelete(BatchDeleteSpec),
    /// 单列等值删除：`DELETE FROM table WHERE key_col = ?`。
    /// 用于按业务作用域丢弃一组派生行（如 too_long 后丢弃某 channel 的本地 message）。
    DeleteWhere(DeleteWhereSpec),
}

/// BatchUpsert 规格
///
/// ## MAJ-5 修复
///
/// `table` 和 `conflict_key` 改为 `&'static str`，消除热路径的运行时 String 分配。
/// 表名和冲突键都是编译期已知的常量（示例 "message"、"temporary_id"——core 不解释含义），
/// `"message".to_string()` 每次消息落库都产生一次堆分配，改为静态引用零成本。
///
/// 注意：`conflict_key` 的具体值（如 `"temporary_id"`）由 ACL-1 在 helix-im 侧提供，
/// helix-core 只传递字符串，不理解其含义。
#[derive(Debug, Clone)]
pub struct UpsertSpec {
    /// 目标表名（编译期常量，如 "message"）
    pub table: &'static str,
    pub rows: Vec<Row>,
    /// ON CONFLICT 的列名（如 ACL-1 提供 "temporary_id"；core 不知道含义）
    pub conflict_key: Option<&'static str>,
    /// 冲突时**不**更新的列（INSERT 仍写其字面值，ON CONFLICT DO UPDATE 排除它们）。
    ///
    /// 用途：守卫列 / 本地维护列——新行用提供的字面值，但已存在的行保留既有值
    /// （"server 缺省 → 回退本地"语义在 upsert 层表达，无需先读后写）。列名编译期常量，
    /// driver 不解释含义（守 HX-C001）。空 = 旧行为（除 conflict_key 外全列更新）。
    ///
    /// 默认空：用 `UpsertSpec::new(...)` 构造可零改动迁移既有调用方。
    pub exclude_from_update: Vec<&'static str>,
}

impl UpsertSpec {
    /// 便捷构造（无排除列，旧默认行为）——既有调用方零改动迁移点。
    pub fn new(table: &'static str, rows: Vec<Row>, conflict_key: Option<&'static str>) -> Self {
        Self {
            table,
            rows,
            conflict_key,
            exclude_from_update: Vec::new(),
        }
    }
}

/// 通用单调 MAX-guard 写规格：仅当 `value > 当前值` 才写入（CAS / 单调寄存器语义）。
///
/// 通用原语，core 不含任何业务含义——典型用途：增量同步水位、presence 心跳时间戳、单调计数器。
/// `scope_key` 是运行时值（由上层提供，如 channel_id.to_string()），core 不解析其含义。
///
/// ## 表名 / 列名作数据（HX-C001）
///
/// `table` / `key_col` / `value_col` / `touch_col` 都是**编译期常量**（`&'static str`），
/// 由上层业务模块提供。core 只把它们当字符串透传给 driver 拼 SQL，**绝不**理解其业务含义
/// ——业务 schema 不进 core（守 HX-C001）。`touch_col` 为 `Some` 时，driver 用自身的
/// ambient 时钟（driver 层合法）写穿该列为当前毫秒时间戳（非权威镜像列，不参与 MAX guard 判定）。
#[derive(Debug, Clone)]
pub struct MonotonicUpsertSpec {
    /// 目标表名（编译期常量；core 不解释含义）
    pub table: &'static str,
    /// PRIMARY KEY 列名（编译期常量）
    pub key_col: &'static str,
    /// 单调 MAX-guard 的值列名（编译期常量）
    pub value_col: &'static str,
    /// 可选「最近更新时间」列名；`Some` 时 driver 写穿当前时间戳，
    /// 不参与 MAX guard。`None` = 无此列（最小单调寄存器形态）。
    pub touch_col: Option<&'static str>,
    /// PK 运行时值（由上层提供）；core 不解析其含义。
    pub scope_key: String,
    /// 单调写入值；仅当 `> 当前值` 才前进。
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

/// BatchDelete 规格：`DELETE FROM table WHERE scope_col = ? AND key_col IN (key_vals)`。
///
/// 复合 PK 作用域删除（如 channel_member：scope=channel_id 等值 + key=user_id IN 列表）——
/// 表名 / 列名编译期常量（core 不解释含义，守 HX-C001），单语句 O(k) 删 k 行。scope 等值约束
/// **绝不跨作用域误删**（无 scope 会把该 user 从所有 channel 删掉）。`key_vals` 空 → 上层应不产 op。
#[derive(Debug, Clone)]
pub struct BatchDeleteSpec {
    /// 目标表名（编译期常量）
    pub table: &'static str,
    /// 作用域列名（等值约束，编译期常量）—— 复合 PK 第一段，如 channel_id。
    pub scope_col: &'static str,
    /// 作用域运行时值（定位作用域；core 不解析含义）
    pub scope_val: SqlValue,
    /// IN-list 列名（编译期常量）—— 复合 PK 第二段，如 user_id。
    pub key_col: &'static str,
    /// 待删行的 key 运行时值列表
    pub key_vals: Vec<SqlValue>,
}

/// DeleteWhere 规格：`DELETE FROM table WHERE key_col = ?`。
///
/// 与 `BatchDelete` 的区别：`BatchDelete` 适合复合 PK 的 scoped IN-list 删除；
/// 本原语适合“按单一作用域列删除该作用域下全部派生行”。表名 / 列名仍是编译期常量，
/// 运行时值由业务模块提供，core 与 driver 不解释业务含义（守 HX-C001）。
#[derive(Debug, Clone)]
pub struct DeleteWhereSpec {
    /// 目标表名（编译期常量）
    pub table: &'static str,
    /// 等值过滤列名（编译期常量）
    pub key_col: &'static str,
    /// 等值过滤运行时值
    pub key_val: SqlValue,
}

/// 守卫式自增规格：单行原子计数器递增 + 可选绝对列写，带前进守卫——**无 read-modify-write**。
///
/// SQL 语义（表名 / 列名都是编译期常量，由上层业务模块提供——守 HX-C001，core/driver
/// 不解释含义；`key_val` / `set_cols` 值 / `bump_delta` / `guard_val` 是运行时数据）：
/// ```sql
/// UPDATE <table> SET <bump_col> = <bump_col> + <bump_delta> [, <set_cols…> = ?…]
///   WHERE <key_col> = ? AND ? > <guard_col>
/// ```
/// （守卫占位 `?` 绑定 `guard_val`：仅当 `guard_val > 当前 guard_col` 才命中更新。）
///
/// ## 为什么需要独立原语（不能用 `BatchUpdate`）
///
/// `BatchUpdate` 只能 `SET col = ?`（绝对值）+ `WHERE key IN (…)`——无法表达
/// `col = col + delta`（自增）也无法表达 `AND ? > guard_col`（前进守卫）。把「读出 +1 写回」
/// 下沉成单条 SQL `col = col + delta` 是 HX-C005 热路径 O(1) 的硬约束（禁应用层 RMW）；
/// 守卫条件让「同一消息重复投递不重复 +1」「乱序旧消息不污染计数」在 SQL 层幂等成立。
///
/// ## 通用性（YAGNI 边界）
///
/// 纯计数器语义，core 不含任何业务含义——典型用途：未读计数 +1（guard=last_post_at 防回退）、
/// 限流桶累加、引用计数。`set_cols` 为空 = 纯自增；`guard_val=i64::MIN`/无意义守卫由上层避免
/// （上层须保证 guard 语义正确）。`set_cols` 列名同样是 `&'static str` 编译期常量。
#[derive(Debug, Clone)]
pub struct GuardedBumpSpec {
    /// 目标表名（编译期常量；core 不解释含义）
    pub table: &'static str,
    /// 主键列名（编译期常量）
    pub key_col: &'static str,
    /// 主键运行时值（定位单行；core 不解析含义）
    pub key_val: SqlValue,
    /// 被自增的数值列名（编译期常量）
    pub bump_col: &'static str,
    /// 自增量（运行时值，通常 +1；负数 = 自减）
    pub bump_delta: i64,
    /// 同一前进守卫下的额外计数列自增。
    ///
    /// 用于 unread/mention/urgent 等多个派生计数必须在同一条新消息守卫下同时推进的场景。
    /// 列名仍是编译期常量；delta 使用绑定参数，不参与 SQL 字符串拼接。
    pub extra_bumps: Vec<(&'static str, i64)>,
    /// 与自增同事务写入的绝对值列（列名编译期常量，值运行时）；空 = 纯自增。
    pub set_cols: Row,
    /// 前进守卫列名（编译期常量）：仅当 `guard_val > <guard_col> 当前值` 才更新整行。
    pub guard_col: &'static str,
    /// 守卫比较值（运行时）：严格大于现值才命中（防重复 / 旧值回退）。
    pub guard_val: i64,
}

/// get 规格（单行；MAJ-5：`table`/`key_col` = `&'static str`）。
#[derive(Debug, Clone)]
pub struct GetSpec {
    pub table: &'static str,
    pub key_col: &'static str,
    pub key_val: SqlValue,
}

/// scan 规格：通用多行扫描读（C3）。on_start 载入单调水位（全表）/ P6 投影查询（等值过滤+降序）。
/// 过滤/排序**可选 additive**（HX-C004）：`filter`/`order_desc_col` 为 None = 旧全表语义（零行为
/// 改变）；`Some((col,val))`→`WHERE col=?`，`Some(c)`→`ORDER BY c DESC`（列名编译期常量 HX-C001）。
#[derive(Debug, Clone)]
pub struct ScanSpec {
    /// 目标表名（编译期常量）
    pub table: &'static str,
    /// 最多返回行数（None = 全表；driver 应设硬上限防御）
    pub limit: Option<u32>,
    /// 可选等值过滤 `WHERE <col> = <val>`（列名编译期常量，值运行时）。None = 不过滤（旧全表）。
    pub filter: Option<(&'static str, SqlValue)>,
    /// 可选降序列 `ORDER BY <col> DESC`（编译期常量）。None = 不排序（旧 rowid 序）。
    pub order_desc_col: Option<&'static str>,
}

/// SQL 值类型（不含数据库具体类型）
#[derive(Debug, Clone)]
pub enum SqlValue {
    Text(String),
    Integer(i64),
    Real(f64),
    /// 二进制大对象（M2：无损携带，禁 BLOB→lossy text 静默损坏）
    Blob(Vec<u8>),
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

/// 本地文件上传请求（driver 读取本地路径并执行真实上传）。
#[derive(Debug, Clone)]
pub struct FileUploadRequest {
    pub local_path: String,
    pub object_key: String,
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub content_type: Option<String>,
    pub size: Option<u64>,
}

/// 文件上传结果（driver 包装后通过 PortReply 回传）。
#[derive(Debug, Clone)]
pub struct FileUploadResponse {
    pub object_key: String,
    pub url: Option<String>,
    pub etag: Option<String>,
}

/// 模块吐出的 I/O 意图声明（tagged union，非法组合不可表达）。
///
/// **执行顺序契约（EFFECT-5）**：同一个 step 产出的 `Vec<Effect>` 之间**无兑现顺序保证**——
/// driver 可乱序/并发兑现（如 Persist 入队即返回，Send/Emit inline await）。需要顺序的因果
/// **必须经 `Correlation` 串联**（吐 Request/Persist{corr} → 等 `PortReply` → 再吐下一步），
/// 不可依赖 `Vec` 下标顺序。
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upload_file_effect_carries_path_not_bytes() {
        let req = FileUploadRequest {
            local_path: "/tmp/a.png".to_string(),
            object_key: "im/a.png".to_string(),
            method: "PUT".to_string(),
            url: "https://upload.example.com/im/a.png".to_string(),
            headers: vec![("x-oss-meta".to_string(), "1".to_string())],
            content_type: Some("image/png".to_string()),
            size: Some(123),
        };

        let effect = Effect::UploadFile {
            corr: Correlation::from_raw(9),
            req,
        };

        match effect {
            Effect::UploadFile { corr, req } => {
                assert_eq!(corr.raw(), 9);
                assert_eq!(req.local_path, "/tmp/a.png");
                assert_eq!(req.object_key, "im/a.png");
                assert_eq!(req.method, "PUT");
                assert_eq!(req.size, Some(123));
            }
            _ => panic!("expected upload effect"),
        }
    }
}
