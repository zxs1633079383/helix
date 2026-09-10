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
    /// 复合作用域守卫式自增：在 scope + key 精确单行内完成幂等计数与绝对列写。
    ScopedGuardedBump(ScopedGuardedBumpSpec),
    /// get：SELECT * FROM table WHERE ...（单行）
    Get(GetSpec),
    /// 复合作用域 get：SELECT * FROM table WHERE scope=? AND key=?（单行）
    ScopedGet(ScopedGetSpec),
    /// scan：SELECT * FROM table [LIMIT n]（多行，C3）
    Scan(ScanSpec),
    /// batch delete：`DELETE FROM table WHERE scope_col = ? AND key_col IN (key_vals)`。
    /// 复合 PK 作用域删除（如 channel_member 成员离场：scope=channel_id + key=user_id）。
    /// O(k) 单语句；scope 等值约束**绝不跨作用域误删**。见 `BatchDeleteSpec`。
    BatchDelete(BatchDeleteSpec),
}

/// 稀疏、非 SQL 存储适配器使用的表身份描述。
///
/// 具体表名由业务模块提供；平台驱动只据此建立通用索引，不依赖业务类型或解析 DDL。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SparseTableSpec {
    pub table: &'static str,
    pub identity_cols: &'static [&'static str],
    pub scope_col: Option<&'static str>,
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
    /// 可选的冲突行更新守卫；仅原行该列 `IS expected` 时允许 `DO UPDATE`。
    ///
    /// 守卫值走绑定参数，不进入 SQL 文本。缺行时仍按普通 INSERT 处理，因此调用方可用
    /// `SqlValue::Null` 表达“没有旧版本”的并发插入守卫。带守卫的规格只允许一个 row。
    pub update_guard: Option<UpsertGuard>,
    /// Optional orderable column: skip conflicting rows whose stored version is greater.
    /// Values must have a consistent SQL type; missing/NULL stored versions admit initialization.
    /// With an update_guard, a skipped row instead returns a conflict and rolls back the transaction.
    pub version_column: Option<&'static str>,
}

/// batch upsert 冲突行的原子更新守卫。
///
/// `column` 是编译期列名；`expected` 是绑定到 `ON CONFLICT ... DO UPDATE ... WHERE`
/// 的运行时值。驱动必须比较冲突前的原行，而不是 `excluded` 新值。
#[derive(Debug, Clone)]
pub struct UpsertGuard {
    /// 冲突原行中参与 `IS` 比较的列名。
    pub column: &'static str,
    /// 冲突原行必须满足的值；`SqlValue::Null` 表示 SQL NULL。
    pub expected: SqlValue,
}

impl UpsertSpec {
    /// 便捷构造（无排除列，旧默认行为）——既有调用方零改动迁移点。
    pub fn new(table: &'static str, rows: Vec<Row>, conflict_key: Option<&'static str>) -> Self {
        Self {
            table,
            rows,
            conflict_key,
            exclude_from_update: Vec::new(),
            version_column: None,
            update_guard: None,
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

/// 复合作用域守卫式自增规格，避免复合主键热路径退化为应用层 read-modify-write。
#[derive(Debug, Clone)]
pub struct ScopedGuardedBumpSpec {
    /// 目标表名（编译期常量；core 不解释业务 schema）。
    pub table: &'static str,
    /// 第一维作用域列名。
    pub scope_col: &'static str,
    /// 第一维作用域运行时值。
    pub scope_val: SqlValue,
    /// 第二维键列名。
    pub key_col: &'static str,
    /// 第二维键运行时值。
    pub key_val: SqlValue,
    /// 被自增的数值列。
    pub bump_col: &'static str,
    /// 自增量；0 表示只应用同一守卫下的绝对列 patch。
    pub bump_delta: i64,
    /// 与计数器同一 SQL 更新的绝对列。
    pub set_cols: Row,
    /// 前进守卫列。
    pub guard_col: &'static str,
    /// 仅严格大于当前守卫值时更新。
    pub guard_val: i64,
}

/// get 规格（单行；MAJ-5：`table`/`key_col` = `&'static str`）。
#[derive(Debug, Clone)]
pub struct GetSpec {
    pub table: &'static str,
    pub key_col: &'static str,
    pub key_val: SqlValue,
}

/// 复合作用域 get 规格，用两个等值键 O(1) 定位复合主键行。
#[derive(Debug, Clone)]
pub struct ScopedGetSpec {
    pub table: &'static str,
    pub scope_col: &'static str,
    pub scope_val: SqlValue,
    pub key_col: &'static str,
    pub key_val: SqlValue,
}

/// scan 规格：通用多行扫描读（C3）。on_start 载入单调水位（全表）/ P6 投影查询（等值过滤+排序）。
/// 过滤/排序**可选 additive**（HX-C004）：`filter` 为 None、`order_by` 为空 = 旧全表语义
/// （零行为改变）；列名仍为编译期常量（HX-C001），方向由结构化枚举表达，禁止调用方拼 SQL。
#[derive(Debug, Clone)]
pub struct ScanSpec {
    /// 目标表名（编译期常量）
    pub table: &'static str,
    /// 最多返回行数（None = 全表；driver 应设硬上限防御）
    pub limit: Option<u32>,
    /// 可选等值过滤 `WHERE <col> = <val>`（列名编译期常量，值运行时）。None = 不过滤（旧全表）。
    pub filter: Option<(&'static str, SqlValue)>,
    /// 结构化排序键，按切片顺序形成 `ORDER BY`；空切片 = 不排序（旧 rowid 序）。
    pub order_by: &'static [ScanOrder],
}

/// Scan 排序方向。driver 只把枚举映射成固定 SQL 关键字，不接受运行时 SQL 片段。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortDirection {
    Asc,
    Desc,
}

/// 单个 Scan 排序键。列名是编译期常量，业务层可安全组合多列确定序。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScanOrder {
    pub column: &'static str,
    pub direction: SortDirection,
}

impl ScanOrder {
    pub const fn asc(column: &'static str) -> Self {
        Self {
            column,
            direction: SortDirection::Asc,
        }
    }

    pub const fn desc(column: &'static str) -> Self {
        Self {
            column,
            direction: SortDirection::Desc,
        }
    }
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
