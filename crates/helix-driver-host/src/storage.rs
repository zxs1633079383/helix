//! Shared rusqlite storage implementation for PC Tauri and Flutter FFI.
//!
//! ## 读写分离（B1 回归修复）
//!
//! 旧实现（commit a0595607 重构后）把 native 三池塌成单个 `Arc<Mutex<Connection>>`，
//! 导致 get/scan 与任何 op 串行在同一把锁 —— ADR-007/engine_loop §3 宣称的「跨组并发」
//! 成了摆设。本实现恢复读写分离：
//!
//! - **1 个写连接**（`Mutex<Connection>`，`SQLITE_OPEN_READ_WRITE | CREATE`）：
//!   batch_upsert / batch_update / monotonic_upsert 路由到这里。SQLite WAL 是单写者，
//!   写串行本就正确，单写连接不损失正确性。
//! - **N=4 个只读连接池**（每个 `SQLITE_OPEN_READ_ONLY` 打开同一 WAL 文件）：
//!   get / scan 路由到这里，可并发（WAL 允许多读者与单写者并发）。
//!   池用 tokio `Semaphore`（限并发到 N）+ `Mutex<Vec<Connection>>`（借还空闲连接）实现。
//!
//! WAL 模式下读者读快照、不阻塞写者，写者也不阻塞读者 → 真并发。
//!
//! ## 不变量（不可破）
//!
//! - batch_upsert 单事务（E2 写放大 1x）
//! - monotonic MAX guard（`ON CONFLICT ... WHERE excluded.value > value`，E5）
//! - `_helix_monotonic` 表在 open()/migrate() 建（不在热路径）
//! - PRAGMA journal_mode=WAL + synchronous=NORMAL + busy_timeout=5000（写、读连接都设）
//! - 连接 flags 用 `SQLITE_OPEN_NO_MUTEX`（B2）：连接已被 Rust Mutex/只读池独占管理，
//!   消除 SQLite 内部串行化锁的双重开销。

use std::sync::{Arc, Mutex};

use helix_core::effect::{
    BatchDeleteSpec, BatchUpdateSpec, GetSpec, GuardedBumpSpec, MonotonicUpsertSpec,
    Row as HelixRow, ScanSpec, SqlValue, UpsertSpec,
};
use helix_core::ports::Storage;
use helix_core::PortError;
use rusqlite::types::{Value, ValueRef};
use rusqlite::{params_from_iter, Connection, OpenFlags};
use tokio::sync::Semaphore;

/// 只读连接池大小（恢复 native 旧三池中「读 ×4」并发度）。
const READ_POOL_SIZE: usize = 4;

/// 内存数据库（`mode=memory` / `:memory:`）无法被第二个连接以 READ_ONLY 重开成同一份
/// 数据（每个 `Connection::open` 得到独立的空库），除非走 `cache=shared`。文件 WAL
/// 库才天然支持多连接共享。因此只读池仅对「可被多连接共享的目标」开启；其余目标
/// （独占内存库等）回退到写连接做读，保证语义正确（牺牲并发度，但这类目标本就无并发需求）。
#[derive(Clone)]
pub struct HostStorage {
    /// 单写连接：所有写 op 串行（WAL 单写者本就如此）。
    writer: Arc<Mutex<Connection>>,
    /// 只读连接池：get/scan 并发借还；None = 目标不支持多连接共享，读回退到 writer。
    readers: Option<Arc<ReadPool>>,
}

/// 只读连接池：Semaphore 限并发 + Mutex<Vec> 借还空闲连接。
struct ReadPool {
    /// 限制同时在用的只读连接数 == 池容量；`acquire().await` 在满载时异步挂起
    /// （不阻塞线程，背压自然传导）。持有 permit 即保证 idle 栈必有一个空闲连接。
    permits: Arc<Semaphore>,
    /// 空闲只读连接栈。permit 持有者保证 pop 必有连接（容量与 permit 数一致）。
    idle: Mutex<Vec<Connection>>,
}

impl HostStorage {
    pub async fn open_sqlite_url(db_url: &str) -> Result<Self, PortError> {
        let target = sqlite_target_from_url(db_url);
        // 只读池仅对可被多连接共享的目标开启（文件库，或显式 cache=shared 的内存库）。
        let shareable = target_supports_shared_readers(&target);

        let target_for_blocking = target.clone();
        let (writer, readers) = tokio::task::spawn_blocking(move || {
            let writer = open_writer(&target_for_blocking)?;

            let readers = if shareable {
                let mut conns = Vec::with_capacity(READ_POOL_SIZE);
                for _ in 0..READ_POOL_SIZE {
                    conns.push(open_reader(&target_for_blocking)?);
                }
                Some(Arc::new(ReadPool {
                    permits: Arc::new(Semaphore::new(READ_POOL_SIZE)),
                    idle: Mutex::new(conns),
                }))
            } else {
                None
            };

            Ok::<_, PortError>((writer, readers))
        })
        .await
        .map_err(map_join_err)??;

        let storage = Self {
            writer: Arc::new(Mutex::new(writer)),
            readers,
        };
        storage.migrate().await?;
        Ok(storage)
    }

    pub async fn execute_raw(&self, sql: &'static str) -> Result<(), PortError> {
        let writer = Arc::clone(&self.writer);
        tokio::task::spawn_blocking(move || {
            let conn = writer.lock().map_err(map_lock_err)?;
            conn.execute_batch(sql).map_err(map_sqlite_err)
        })
        .await
        .map_err(map_join_err)?
    }

    async fn migrate(&self) -> Result<(), PortError> {
        // 写连接建表 + 设 PRAGMA。WAL 是库级持久属性，读连接 open 后亦生效。
        self.execute_raw(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=NORMAL;
             PRAGMA busy_timeout=5000;
             CREATE TABLE IF NOT EXISTS _helix_monotonic
             (scope_key TEXT PRIMARY KEY NOT NULL, value INTEGER NOT NULL DEFAULT 0);",
        )
        .await
    }

    /// 在只读连接上跑闭包（无可共享只读池时回退到写连接）。
    ///
    /// get/scan 走这里 → WAL 下可与写者及其他读者并发。
    async fn with_reader<T, F>(&self, f: F) -> Result<T, PortError>
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> Result<T, PortError> + Send + 'static,
    {
        match &self.readers {
            Some(pool) => {
                // 先 async 取 permit（满载时异步挂起，不占线程，背压自然传导）；
                // 持有 permit ⇒ idle 栈必有一个空闲连接可借。
                let permit = Arc::clone(&pool.permits)
                    .acquire_owned()
                    .await
                    .map_err(|_| PortError::Storage("read pool semaphore closed".to_string()))?;
                let pool = Arc::clone(pool);
                tokio::task::spawn_blocking(move || {
                    let conn = {
                        let mut idle = pool.idle.lock().map_err(map_lock_err)?;
                        idle.pop()
                    };
                    let Some(conn) = conn else {
                        // permit 与连接数一致，理论不可达；防御性报错而非 panic。
                        return Err(PortError::Storage(
                            "read pool permit/conn mismatch".to_string(),
                        ));
                    };
                    let out = f(&conn);
                    // 归还连接（即便 f 出错连接仍可用），随后 drop permit 释放并发额度。
                    if let Ok(mut idle) = pool.idle.lock() {
                        idle.push(conn);
                    }
                    drop(permit);
                    out
                })
                .await
                .map_err(map_join_err)?
            }
            None => {
                // 无可共享只读池（独占内存库等）→ 在写连接上读，串行但正确。
                let writer = Arc::clone(&self.writer);
                tokio::task::spawn_blocking(move || {
                    let conn = writer.lock().map_err(map_lock_err)?;
                    f(&conn)
                })
                .await
                .map_err(map_join_err)?
            }
        }
    }
}

#[async_trait::async_trait]
impl Storage for HostStorage {
    async fn batch_upsert(&self, spec: UpsertSpec) -> Result<(), PortError> {
        if spec.rows.is_empty() {
            return Ok(());
        }

        let writer = Arc::clone(&self.writer);
        tokio::task::spawn_blocking(move || {
            let mut conn = writer.lock().map_err(map_lock_err)?;
            // HX-C005 TODO（不过度工程）：batch_upsert 的 SQL 因 table + 动态列名 + conflict_key
            // 形状各异，单 `&'static str` 不可行。若实测此路径成为分配热点，可按
            // (table, col_count, has_conflict_key) 形状缓存 SQL 串（OnceCell/HashMap）。
            // 现状：prepare_cached 已复用已编译语句，每次仅重建 SQL 字符串本身——
            // batch_upsert 是「整批一事务」而非 per-message 单条，相对 monotonic cursor
            // 冷得多，暂不缓存。
            let col_names: Vec<&str> = spec.rows[0].iter().map(|(k, _)| k.as_str()).collect();
            let cols_sql = col_names.join(", ");
            let placeholders = col_names.iter().map(|_| "?").collect::<Vec<_>>().join(", ");

            let sql = match spec.conflict_key {
                Some(key) => {
                    // ON CONFLICT 更新集排除：conflict_key 自身 + 调用方声明的 exclude_from_update
                    // （守卫 / 本地维护列——冲突时保留既有值 = "回退本地"，HX-C005 无需先读后写）。
                    let update_cols: Vec<String> = col_names
                        .iter()
                        .filter(|&&c| c != key && !spec.exclude_from_update.contains(&c))
                        .map(|c| format!("{c} = excluded.{c}"))
                        .collect();
                    if update_cols.is_empty() {
                        format!(
                            "INSERT INTO {table} ({cols}) VALUES ({placeholders}) ON CONFLICT({key}) DO NOTHING",
                            table = spec.table,
                            cols = cols_sql,
                            placeholders = placeholders,
                        )
                    } else {
                        format!(
                            "INSERT INTO {table} ({cols}) VALUES ({placeholders}) ON CONFLICT({key}) DO UPDATE SET {updates}",
                            table = spec.table,
                            cols = cols_sql,
                            placeholders = placeholders,
                            updates = update_cols.join(", "),
                        )
                    }
                }
                None => format!(
                    "INSERT OR REPLACE INTO {table} ({cols}) VALUES ({placeholders})",
                    table = spec.table,
                    cols = cols_sql,
                    placeholders = placeholders,
                ),
            };

            // E2 不变量：整批一事务，写放大 1x。中途失败 → 事务回滚（tx drop 未 commit 即回滚），整批不落。
            let tx = conn.transaction().map_err(map_sqlite_err)?;
            {
                let mut stmt = tx.prepare_cached(&sql).map_err(map_sqlite_err)?;
                for row in &spec.rows {
                    let values = sql_values(row.iter().map(|(_, value)| value));
                    stmt.execute(params_from_iter(values.iter()))
                        .map_err(map_sqlite_err)?;
                }
            }
            tx.commit().map_err(map_sqlite_err)
        })
        .await
        .map_err(map_join_err)?
    }

    async fn batch_update(&self, spec: BatchUpdateSpec) -> Result<(), PortError> {
        let BatchUpdateSpec {
            table,
            key_col,
            key_vals,
            patch,
        } = spec;
        if key_vals.is_empty() || patch.is_empty() {
            return Ok(());
        }

        let writer = Arc::clone(&self.writer);
        let table = table.to_string();
        let key_col = key_col.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = writer.lock().map_err(map_lock_err)?;
            let set_clause: Vec<String> = patch.iter().map(|(k, _)| format!("{k} = ?")).collect();
            let in_placeholders: Vec<&str> = key_vals.iter().map(|_| "?").collect();
            let sql = format!(
                "UPDATE {table} SET {set} WHERE {key_col} IN ({in_vals})",
                set = set_clause.join(", "),
                in_vals = in_placeholders.join(", "),
            );

            let mut values = sql_values(patch.iter().map(|(_, value)| value));
            values.extend(sql_values(key_vals.iter()));
            conn.execute(&sql, params_from_iter(values.iter()))
                .map(|_| ())
                .map_err(map_sqlite_err)
        })
        .await
        .map_err(map_join_err)?
    }

    async fn monotonic_upsert(&self, spec: MonotonicUpsertSpec) -> Result<(), PortError> {
        // HX-C005 热路径零分配 / O(1)：cursor 推进是每条 WS 消息触发的最热路径。
        // SQL 由 spec 的 `&'static str` 表名 / 列名拼装（表名 / 列名都是编译期常量，
        // 由上层业务模块提供——守 HX-C001，core/driver 不解释含义），配合 prepare_cached
        // 复用已编译语句（同一组常量 → 同一 SQL 串 → 同一 cache 槽），消除 per-call parse 开销。
        // 单条 INSERT … ON CONFLICT DO UPDATE = O(1)，无 read-modify-write。
        //
        // MAX guard（E5 / S2 不变量）：`WHERE excluded.value > <table>.<value_col>` —— 仅当严格
        // 大于现值才前进（防 cursor 回退）。`touch_col`（如 updated_at）由本 driver 的 ambient
        // 时钟写穿（driver 层合法，见本 crate 不变量 #5），**不**参与 MAX guard——它是非权威镜像
        // 列；即便 MAX guard 因无进展不更新 value，updated_at 仍随 excluded 覆盖（对齐现网
        // channel_event_cursor.rs:55-56 `updated_at = excluded.updated_at`）。
        //
        // HX-C005 零分配：唯一热 shape（advance_cursor：channel_event_cursor / channel_id /
        // last_event_seq / updated_at）→ `&'static str` 常量 SQL，per-call **不分配 String**
        // （`Cow::Borrowed`）。其余 shape（当前运行时无调用，保泛型 API）才走 `format!` 兜底。
        // 对齐现网 channel_event_cursor.rs:54-56：value 走 MAX guard（不回退）；touch（updated_at）
        // 恒随 excluded 覆盖（MAX 已防 value 回退，updated_at 每写刷新，与现网 Utc::now 写穿一致）。
        const CURSOR_ADVANCE_UPSERT_SQL: &str =
            "INSERT INTO channel_event_cursor (channel_id, last_event_seq, updated_at) \
             VALUES (?, ?, ?) ON CONFLICT(channel_id) DO UPDATE SET \
             last_event_seq = MAX(channel_event_cursor.last_event_seq, excluded.last_event_seq), \
             updated_at = excluded.updated_at";
        let sql: std::borrow::Cow<'static, str> =
            match (spec.table, spec.key_col, spec.value_col, spec.touch_col) {
                ("channel_event_cursor", "channel_id", "last_event_seq", Some("updated_at")) => {
                    std::borrow::Cow::Borrowed(CURSOR_ADVANCE_UPSERT_SQL)
                }
                (table, key, val, Some(touch)) => std::borrow::Cow::Owned(format!(
                    "INSERT INTO {table} ({key}, {val}, {touch}) VALUES (?, ?, ?) \
                     ON CONFLICT({key}) DO UPDATE SET \
                     {val} = MAX({table}.{val}, excluded.{val}), {touch} = excluded.{touch}",
                )),
                (table, key, val, None) => std::borrow::Cow::Owned(format!(
                    "INSERT INTO {table} ({key}, {val}) VALUES (?, ?) \
                     ON CONFLICT({key}) DO UPDATE SET {val} = excluded.{val} \
                     WHERE excluded.{val} > {table}.{val}",
                )),
            };
        // updated_at 由 driver ambient 时钟提供（毫秒，对齐现网 Utc::now().timestamp_millis()）。
        // 仅 touch_col=Some 才需 → 无 touch 列不调 SystemTime::now() syscall（HX-C005）。
        let now_ms = match spec.touch_col {
            Some(_) => std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0),
            None => 0,
        };
        let writer = Arc::clone(&self.writer);
        tokio::task::spawn_blocking(move || {
            let conn = writer.lock().map_err(map_lock_err)?;
            let mut stmt = conn.prepare_cached(sql.as_ref()).map_err(map_sqlite_err)?;
            match spec.touch_col {
                Some(_) => stmt.execute((&spec.scope_key, spec.value, now_ms)),
                None => stmt.execute((&spec.scope_key, spec.value)),
            }
            .map(|_| ())
            .map_err(map_sqlite_err)
        })
        .await
        .map_err(map_join_err)?
    }

    async fn guarded_bump(&self, spec: GuardedBumpSpec) -> Result<(), PortError> {
        // HX-C005 热路径 O(1)：未读 +1 等计数路径——单条 UPDATE WHERE PK，**无 read-modify-write**。
        // SQL：`SET bump_col = bump_col + ? [, set_cols = ?…] WHERE key = ? AND ? > guard_col`。
        // 守卫占位 `?` 绑定 guard_val（严格 > guard_col 现值才命中）——同一消息重复投递 / 旧消息
        // 乱序到达均在 SQL 层幂等不污染计数。表名 / 列名都是 spec 的 `&'static str`（上层编译期
        // 常量，driver 不解释含义，守 HX-C001），prepare_cached 复用同形状语句。
        let writer = Arc::clone(&self.writer);
        tokio::task::spawn_blocking(move || {
            // SET 片段：先自增列（delta 走**绑定参数** `?`，非内插 SQL 文本——HX-C005：SQL 文本
            // 不随 bump_delta 值变 → prepare_cached 同形状命中，零 per-delta 重编译），再可选绝对列。
            let mut set_parts = vec![format!("{bump} = {bump} + ?", bump = spec.bump_col)];
            set_parts.extend(spec.set_cols.iter().map(|(k, _)| format!("{k} = ?")));
            let sql = format!(
                "UPDATE {table} SET {set} WHERE {key} = ? AND ? > {guard}",
                table = spec.table,
                set = set_parts.join(", "),
                key = spec.key_col,
                guard = spec.guard_col,
            );
            // 绑定顺序与 SQL 占位一致：bump_delta → set_cols 值… → key_val → guard_val。
            let mut params = sql_values(std::iter::once(&SqlValue::Integer(spec.bump_delta)));
            params.extend(sql_values(spec.set_cols.iter().map(|(_, v)| v)));
            params.extend(sql_values(std::iter::once(&spec.key_val)));
            params.extend(sql_values(std::iter::once(&SqlValue::Integer(
                spec.guard_val,
            ))));
            let conn = writer.lock().map_err(map_lock_err)?;
            let mut stmt = conn.prepare_cached(&sql).map_err(map_sqlite_err)?;
            stmt.execute(params_from_iter(params.iter()))
                .map(|_| ())
                .map_err(map_sqlite_err)
        })
        .await
        .map_err(map_join_err)?
    }

    async fn get(&self, spec: GetSpec) -> Result<Option<HelixRow>, PortError> {
        self.with_reader(move |conn| {
            let sql = format!(
                "SELECT * FROM {table} WHERE {key_col} = ?",
                table = spec.table,
                key_col = spec.key_col,
            );
            let values = sql_values(std::iter::once(&spec.key_val));
            let mut stmt = conn.prepare_cached(&sql).map_err(map_sqlite_err)?;
            let mut rows = stmt
                .query(params_from_iter(values.iter()))
                .map_err(map_sqlite_err)?;
            match rows.next().map_err(map_sqlite_err)? {
                Some(row) => convert_row(row).map(Some),
                None => Ok(None),
            }
        })
        .await
    }

    async fn scan(&self, spec: ScanSpec) -> Result<Vec<HelixRow>, PortError> {
        const HARD_LIMIT: u32 = 10_000;
        self.with_reader(move |conn| {
            let limit = spec.limit.unwrap_or(HARD_LIMIT).min(HARD_LIMIT);
            // 可选过滤/排序（P6 投影；None=旧全表语义零行为改变）。列名 core 编译期 `&'static str`
            // （HX-C001 列名作数据）；过滤值走绑定参数防注入。
            let where_clause = spec
                .filter
                .as_ref()
                .map_or(String::new(), |(c, _)| format!(" WHERE {c} = ?"));
            let order_clause = spec
                .order_desc_col
                .map_or(String::new(), |c| format!(" ORDER BY {c} DESC"));
            let sql = format!(
                "SELECT * FROM {table}{where_clause}{order_clause} LIMIT {limit}",
                table = spec.table,
            );
            let bind: Vec<Value> = spec
                .filter
                .as_ref()
                .map_or_else(Vec::new, |(_, val)| sql_values(std::iter::once(val)));
            let mut stmt = conn.prepare_cached(&sql).map_err(map_sqlite_err)?;
            let mut rows = stmt
                .query(params_from_iter(bind.iter()))
                .map_err(map_sqlite_err)?;
            let mut out = Vec::new();
            while let Some(row) = rows.next().map_err(map_sqlite_err)? {
                out.push(convert_row(row)?);
            }
            Ok(out)
        })
        .await
    }

    async fn batch_delete(&self, spec: BatchDeleteSpec) -> Result<(), PortError> {
        let BatchDeleteSpec {
            table,
            scope_col,
            scope_val,
            key_col,
            key_vals,
        } = spec;
        if key_vals.is_empty() {
            return Ok(());
        }

        let writer = Arc::clone(&self.writer);
        let table = table.to_string();
        let scope_col = scope_col.to_string();
        let key_col = key_col.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = writer.lock().map_err(map_lock_err)?;
            let in_placeholders: Vec<&str> = key_vals.iter().map(|_| "?").collect();
            // DELETE FROM <table> WHERE <scope_col> = ? AND <key_col> IN (?…)
            // scope 等值约束（复合 PK 第一段）定位单作用域，绝不跨作用域误删；O(k) 单语句。
            let sql = format!(
                "DELETE FROM {table} WHERE {scope_col} = ? AND {key_col} IN ({in_vals})",
                in_vals = in_placeholders.join(", "),
            );
            // 绑定顺序与占位一致：scope_val → key_vals…
            let mut values = sql_values(std::iter::once(&scope_val));
            values.extend(sql_values(key_vals.iter()));
            conn.execute(&sql, params_from_iter(values.iter()))
                .map(|_| ())
                .map_err(map_sqlite_err)
        })
        .await
        .map_err(map_join_err)?
    }
}

/// 打开写连接：READ_WRITE | CREATE，NO_MUTEX（B2，连接被 Rust Mutex 独占）+ URI。
fn open_writer(target: &str) -> Result<Connection, PortError> {
    let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
        | OpenFlags::SQLITE_OPEN_CREATE
        | OpenFlags::SQLITE_OPEN_URI
        | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    Connection::open_with_flags(target, flags).map_err(map_sqlite_err)
}

/// 打开只读连接：READ_ONLY（同一 WAL 文件可多连接并发读）+ NO_MUTEX（B2，连接被只读池独占）+ URI。
fn open_reader(target: &str) -> Result<Connection, PortError> {
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY
        | OpenFlags::SQLITE_OPEN_URI
        | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let conn = Connection::open_with_flags(target, flags).map_err(map_sqlite_err)?;
    // 读连接也设 busy_timeout，避免极短窗口内 WAL checkpoint 引起的瞬时 SQLITE_BUSY。
    conn.busy_timeout(std::time::Duration::from_millis(5000))
        .map_err(map_sqlite_err)?;
    Ok(conn)
}

/// 判断目标是否支持多连接共享同一份数据（决定是否启用只读池）。
///
/// - 文件库（含 `file:...` URI 指向真实路径、普通路径）→ 支持。
/// - `cache=shared` 的内存库 → 支持（多连接共享同一内存页缓存）。
/// - 独占内存库（`:memory:` / `mode=memory` 无 `cache=shared`）→ 不支持，
///   第二个连接会得到独立空库，必须回退到写连接做读。
fn target_supports_shared_readers(target: &str) -> bool {
    let is_memory = target.contains(":memory:") || target.contains("mode=memory");
    if !is_memory {
        // 文件库（或临时文件）天然支持多连接 WAL 并发读。
        return true;
    }
    // 内存库仅在显式 cache=shared 时可被多连接共享。
    target.contains("cache=shared")
}

fn sqlite_target_from_url(db_url: &str) -> String {
    let Some(rest) = db_url.strip_prefix("sqlite:") else {
        return db_url.to_string();
    };

    if rest.starts_with("file:") {
        return rest.to_string();
    }

    // M1 修复：不在 '?' 处截断 URL —— 保留 query（?cache=shared / ?mode=memory /
    // 未来 ?vfs= 等）并连同 SQLITE_OPEN_URI 一起透传给 Connection::open_with_flags。
    // 只做纯路径前缀的 '///' → '/' 归一（不碰 query 段）。
    let (path, query) = match rest.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (rest, None),
    };

    let normalized_path = if path.starts_with("///") {
        format!("/{}", path.trim_start_matches('/'))
    } else {
        path.to_string()
    };

    match query {
        Some(q) => format!("{normalized_path}?{q}"),
        None => normalized_path,
    }
}

fn sql_values<'a>(values: impl IntoIterator<Item = &'a SqlValue>) -> Vec<Value> {
    values
        .into_iter()
        .map(|value| match value {
            SqlValue::Text(s) => Value::Text(s.clone()),
            SqlValue::Integer(i) => Value::Integer(*i),
            SqlValue::Real(f) => Value::Real(*f),
            SqlValue::Blob(b) => Value::Blob(b.clone()),
            SqlValue::Null => Value::Null,
        })
        .collect()
}

fn convert_row(row: &rusqlite::Row<'_>) -> Result<HelixRow, PortError> {
    let row_ref = row.as_ref();
    let mut out = Vec::with_capacity(row_ref.column_count());
    for idx in 0..row_ref.column_count() {
        let name = row_ref
            .column_name(idx)
            .map_err(map_sqlite_err)?
            .to_string();
        let value = match row.get_ref(idx).map_err(map_sqlite_err)? {
            ValueRef::Null => SqlValue::Null,
            ValueRef::Integer(i) => SqlValue::Integer(i),
            ValueRef::Real(f) => SqlValue::Real(f),
            ValueRef::Text(s) => SqlValue::Text(String::from_utf8_lossy(s).into_owned()),
            // M2：BLOB 无损携带为 SqlValue::Blob，禁 from_utf8_lossy 静默损坏二进制。
            ValueRef::Blob(b) => SqlValue::Blob(b.to_vec()),
        };
        out.push((name, value));
    }
    Ok(out)
}

fn map_sqlite_err(e: impl std::fmt::Display) -> PortError {
    PortError::Storage(e.to_string())
}

fn map_lock_err<T>(_: std::sync::PoisonError<T>) -> PortError {
    PortError::Storage("sqlite connection lock poisoned".to_string())
}

fn map_join_err(e: tokio::task::JoinError) -> PortError {
    PortError::Storage(format!("sqlite blocking worker failed: {e}"))
}
