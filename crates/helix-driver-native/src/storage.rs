//! NativeStorage — sqlx::SqlitePool (WAL)
//!
//! ## WAL 三池配置
//!
//! - sync_write_pool: max=1（单调 monotonic_upsert；单写者，WAL 文件不锁竞争）
//! - user_write_pool: max=1（业务写 batch_upsert / batch_update；同上）
//! - read_pool:       max=4（并发读 get / scan）
//!
//! 三者共用同一 WAL 文件（SQLite WAL 支持 1 writer + N readers 并发）。
//! busy_timeout=5s 防止写池短暂锁争用时立即失败。
//!
//! ## 动态 SQL 防注入约定
//!
//! 表名 / 列名来自 `&'static str`（编译期常量，不来自用户输入），
//! 直接插值进 SQL 模板是安全的。运行时值（行数据）全部走 `?` 绑定参数。
//!
//! ## rows→bytes 序列化约定（scan 返回多行）
//!
//! `scan` 返回 `Vec<Row>`（Row = Vec<(String, SqlValue)>），直接在 trait 层返回，
//! driver 内部不做额外序列化。engine_loop 需要把 Vec<Row> 放进 PortReply 时，
//! 统一用 `serde_json::to_vec(&rows_as_json_array)` → `Bytes`（在 engine_loop.rs
//! 中定义的 `rows_to_reply_bytes` 辅助函数），JSON schema 为：
//! `[{"col1": val1, "col2": val2}, ...]`，SqlValue 映射为：
//!   Text(s) → JSON string, Integer(i) → JSON number,
//!   Real(f) → JSON number, Null → null。
//! 此约定写在 engine_loop.rs 的 `rows_to_reply_bytes`/`rows_from_reply_bytes` 中。

use helix_core::effect::{
    GetSpec, MonotonicUpsertSpec, Row as HelixRow, ScanSpec, SqlValue, UpsertSpec,
};
use helix_core::ports::Storage;
use helix_core::PortError;
use sqlx::{sqlite::SqlitePoolOptions, Column, SqlitePool};

/// SQLite WAL 驱动存储。
///
/// 三池（sync 写 + 用户写 + 读）共享同一 SQLite WAL 文件。
#[derive(Clone)]
pub struct NativeStorage {
    /// 单调写（monotonic_upsert）—— max_connections=1
    sync_write_pool: SqlitePool,
    /// 业务写（batch_upsert / batch_update）—— max_connections=1
    user_write_pool: SqlitePool,
    /// 并发读（get / scan）—— max_connections=4
    read_pool: SqlitePool,
}

impl NativeStorage {
    /// 打开或创建 SQLite 数据库，建立三池。
    ///
    /// `db_url` 格式：`sqlite:///path/to/helix.db`（含 `?mode=rwc` 自动创建）。
    pub async fn open(db_url: &str) -> Result<Self, PortError> {
        let sync_write_pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect(db_url)
            .await
            .map_err(Self::map_err)?;

        // WAL 模式 + busy_timeout（防短暂锁争用立即失败）
        sqlx::query("PRAGMA journal_mode=WAL")
            .execute(&sync_write_pool)
            .await
            .map_err(Self::map_err)?;
        sqlx::query("PRAGMA busy_timeout=5000")
            .execute(&sync_write_pool)
            .await
            .map_err(Self::map_err)?;

        let user_write_pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect(db_url)
            .await
            .map_err(Self::map_err)?;

        sqlx::query("PRAGMA journal_mode=WAL")
            .execute(&user_write_pool)
            .await
            .map_err(Self::map_err)?;
        sqlx::query("PRAGMA busy_timeout=5000")
            .execute(&user_write_pool)
            .await
            .map_err(Self::map_err)?;

        let read_pool = SqlitePoolOptions::new()
            .max_connections(4)
            .connect(db_url)
            .await
            .map_err(Self::map_err)?;

        sqlx::query("PRAGMA busy_timeout=5000")
            .execute(&read_pool)
            .await
            .map_err(Self::map_err)?;

        Ok(Self {
            sync_write_pool,
            user_write_pool,
            read_pool,
        })
    }

    /// sqlx::Error → PortError::Storage（ACL-2 翻译）
    fn map_err(e: impl std::fmt::Display) -> PortError {
        PortError::Storage(e.to_string())
    }

    /// `SqlValue` → sqlx 绑定参数（注入安全：运行时值走 ? 绑定）
    fn bind_value<'q>(
        q: sqlx::query::Query<'q, sqlx::Sqlite, sqlx::sqlite::SqliteArguments<'q>>,
        v: &'q SqlValue,
    ) -> sqlx::query::Query<'q, sqlx::Sqlite, sqlx::sqlite::SqliteArguments<'q>> {
        match v {
            SqlValue::Text(s) => q.bind(s.as_str()),
            SqlValue::Integer(i) => q.bind(*i),
            SqlValue::Real(f) => q.bind(*f),
            SqlValue::Null => q.bind(Option::<String>::None),
        }
    }

    /// sqlx AnyRow → helix Row（列名+值）
    fn convert_row(row: &sqlx::sqlite::SqliteRow) -> HelixRow {
        use sqlx::Row as _;
        let cols = row.columns();
        cols.iter()
            .map(|col| {
                let name = col.name().to_string();
                // 按 SQLite type affinity 顺序尝试：Integer → Real → Text → Null
                let val: SqlValue = if let Ok(i) = row.try_get::<i64, _>(col.ordinal()) {
                    SqlValue::Integer(i)
                } else if let Ok(f) = row.try_get::<f64, _>(col.ordinal()) {
                    SqlValue::Real(f)
                } else if let Ok(s) = row.try_get::<String, _>(col.ordinal()) {
                    SqlValue::Text(s)
                } else {
                    SqlValue::Null
                };
                (name, val)
            })
            .collect()
    }
}

#[async_trait::async_trait]
impl Storage for NativeStorage {
    /// INSERT INTO {table} ({cols}) VALUES (?) ON CONFLICT ({conflict_key}) DO UPDATE SET {patch}
    ///
    /// 动态拼列名（来自 &'static str 或编译期常量 String 键，非用户输入），
    /// 运行时行数据全部走 ? 绑定，防注入。
    ///
    /// ## 显式事务
    ///
    /// 整批 N 行包进一个 `BEGIN … COMMIT`，只触发一次 WAL fsync，相比逐行
    /// autocommit 写放大消除约 9x（实测：369ms → ~40ms / 5000 行）。
    /// 中途失败 → tx drop 自动 ROLLBACK，返回 Err；调用方 cursor 不推进，
    /// 下次重发整批（幂等 upsert 安全）。
    async fn batch_upsert(&self, spec: UpsertSpec) -> Result<(), PortError> {
        if spec.rows.is_empty() {
            return Ok(());
        }

        // 从第一行推断列名（所有行列名相同，保证调用方一致性）
        let col_names: Vec<&str> = spec.rows[0].iter().map(|(k, _)| k.as_str()).collect();
        let cols_sql = col_names.join(", ");
        let placeholders = col_names.iter().map(|_| "?").collect::<Vec<_>>().join(", ");

        // SQL 字符串在循环外 format 一次复用（避免每行 allocate）
        let sql = match spec.conflict_key {
            Some(key) => {
                // ON CONFLICT(key) DO UPDATE SET col = excluded.col, ...（排除冲突键本身）
                let update_cols: Vec<String> = col_names
                    .iter()
                    .filter(|&&c| c != key)
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

        // 整批包进一个显式事务：N 行 = 1 次 WAL fsync（修 E2）
        // drop-on-error 自动 ROLLBACK，不需要显式 rollback 调用
        let mut tx = self.user_write_pool.begin().await.map_err(Self::map_err)?;
        for row in &spec.rows {
            let mut q = sqlx::query(&sql);
            for (_, val) in row {
                q = Self::bind_value(q, val);
            }
            q.execute(&mut *tx).await.map_err(Self::map_err)?;
        }
        tx.commit().await.map_err(Self::map_err)?;

        Ok(())
    }

    /// UPDATE {table} SET {patch} WHERE {key_col} IN (?, ?, ...)
    async fn batch_update(
        &self,
        table: &str,
        key_col: &str,
        key_vals: &[SqlValue],
        patch: HelixRow,
    ) -> Result<(), PortError> {
        if key_vals.is_empty() || patch.is_empty() {
            return Ok(());
        }

        let set_clause: Vec<String> = patch.iter().map(|(k, _)| format!("{k} = ?")).collect();
        let in_placeholders: Vec<&str> = key_vals.iter().map(|_| "?").collect();

        let sql = format!(
            "UPDATE {table} SET {set} WHERE {key_col} IN ({in_vals})",
            table = table,
            set = set_clause.join(", "),
            key_col = key_col,
            in_vals = in_placeholders.join(", "),
        );

        let mut q = sqlx::query(&sql);
        // patch 值先绑定（SET 子句）
        for (_, val) in &patch {
            q = Self::bind_value(q, val);
        }
        // 再绑定 WHERE IN 值
        for val in key_vals {
            q = Self::bind_value(q, val);
        }
        q.execute(&self.user_write_pool)
            .await
            .map_err(Self::map_err)?;

        Ok(())
    }

    /// INSERT INTO _helix_monotonic (scope_key, value) VALUES (?, ?)
    /// ON CONFLICT(scope_key) DO UPDATE SET value = excluded.value
    ///   WHERE excluded.value > _helix_monotonic.value
    ///
    /// 专用表 `_helix_monotonic`（driver 约定，core 不知道表名）。
    /// O(1) per call：单行 WHERE PK lookup。
    async fn monotonic_upsert(&self, spec: MonotonicUpsertSpec) -> Result<(), PortError> {
        // 确保表存在（首次调用时建表，幂等）
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS _helix_monotonic \
             (scope_key TEXT PRIMARY KEY NOT NULL, value INTEGER NOT NULL DEFAULT 0)",
        )
        .execute(&self.sync_write_pool)
        .await
        .map_err(Self::map_err)?;

        sqlx::query(
            "INSERT INTO _helix_monotonic (scope_key, value) VALUES (?, ?) \
             ON CONFLICT(scope_key) DO UPDATE SET value = excluded.value \
             WHERE excluded.value > _helix_monotonic.value",
        )
        .bind(&spec.scope_key)
        .bind(spec.value)
        .execute(&self.sync_write_pool)
        .await
        .map_err(Self::map_err)?;

        Ok(())
    }

    /// SELECT * FROM {table} WHERE {key_col} = ?（读池）
    async fn get(&self, spec: GetSpec) -> Result<Option<HelixRow>, PortError> {
        let sql = format!(
            "SELECT * FROM {table} WHERE {key_col} = ?",
            table = spec.table,
            key_col = spec.key_col,
        );
        let mut q = sqlx::query(&sql);
        q = Self::bind_value(q, &spec.key_val);

        let row = q
            .fetch_optional(&self.read_pool)
            .await
            .map_err(Self::map_err)?;
        Ok(row.map(|r| Self::convert_row(&r)))
    }

    /// SELECT * FROM {table} LIMIT {limit}（读池，硬上限 10000）
    async fn scan(&self, spec: ScanSpec) -> Result<Vec<HelixRow>, PortError> {
        const HARD_LIMIT: u32 = 10_000;
        let limit = spec.limit.unwrap_or(HARD_LIMIT).min(HARD_LIMIT);

        let sql = format!("SELECT * FROM {table} LIMIT {limit}", table = spec.table);
        let rows = sqlx::query(&sql)
            .fetch_all(&self.read_pool)
            .await
            .map_err(Self::map_err)?;

        Ok(rows.iter().map(Self::convert_row).collect())
    }
}

// ─── 单元测试（内存 SQLite，不依赖文件系统）────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use helix_core::effect::{GetSpec, MonotonicUpsertSpec, ScanSpec, SqlValue, UpsertSpec};

    async fn in_memory_storage() -> NativeStorage {
        // sqlite::memory: 每个连接是独立的内存 DB；用命名共享内存让三池共享同一 DB。
        // 注意：测试之间可能共享同一命名内存，但三池内各连接共享同一 DB 是必要的。
        // `?cache=shared` 让同进程内同名内存 DB 被多个连接共享（SQLite URI mode）。
        // sqlx 用 `sqlite:` scheme，URI 参数需要 `?mode=memory&cache=shared`。
        // 最简单方案：三池都用 max_connections=1 连同一个命名 in-memory DB。
        NativeStorage::open("sqlite:file::testmem:?mode=memory&cache=shared")
            .await
            .expect("open shared in-memory sqlite")
    }

    async fn setup_test_table(storage: &NativeStorage) {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS messages \
             (id INTEGER PRIMARY KEY, content TEXT NOT NULL, seq INTEGER NOT NULL DEFAULT 0)",
        )
        .execute(&storage.user_write_pool)
        .await
        .expect("create test table");
    }

    #[tokio::test]
    async fn test_batch_upsert_insert_and_conflict() {
        let storage = in_memory_storage().await;
        setup_test_table(&storage).await;

        // 首次插入
        let spec = UpsertSpec {
            table: "messages",
            rows: vec![vec![
                ("id".to_string(), SqlValue::Integer(1)),
                ("content".to_string(), SqlValue::Text("hello".to_string())),
                ("seq".to_string(), SqlValue::Integer(1)),
            ]],
            conflict_key: Some("id"),
        };
        storage.batch_upsert(spec).await.expect("first insert");

        // 确认写入
        let row = storage
            .get(GetSpec {
                table: "messages",
                key_col: "id",
                key_val: SqlValue::Integer(1),
            })
            .await
            .expect("get")
            .expect("should exist");
        let content = row.iter().find(|(k, _)| k == "content").unwrap();
        assert!(matches!(&content.1, SqlValue::Text(s) if s == "hello"));

        // ON CONFLICT DO UPDATE（相同 id，更新 content）
        let spec2 = UpsertSpec {
            table: "messages",
            rows: vec![vec![
                ("id".to_string(), SqlValue::Integer(1)),
                ("content".to_string(), SqlValue::Text("world".to_string())),
                ("seq".to_string(), SqlValue::Integer(2)),
            ]],
            conflict_key: Some("id"),
        };
        storage.batch_upsert(spec2).await.expect("upsert conflict");

        let row2 = storage
            .get(GetSpec {
                table: "messages",
                key_col: "id",
                key_val: SqlValue::Integer(1),
            })
            .await
            .expect("get2")
            .expect("should exist");
        let content2 = row2.iter().find(|(k, _)| k == "content").unwrap();
        assert!(
            matches!(&content2.1, SqlValue::Text(s) if s == "world"),
            "upsert should update content"
        );
    }

    #[tokio::test]
    async fn test_batch_update() {
        let storage = in_memory_storage().await;
        setup_test_table(&storage).await;

        // 先插入两行
        let spec = UpsertSpec {
            table: "messages",
            rows: vec![
                vec![
                    ("id".to_string(), SqlValue::Integer(10)),
                    ("content".to_string(), SqlValue::Text("a".to_string())),
                    ("seq".to_string(), SqlValue::Integer(0)),
                ],
                vec![
                    ("id".to_string(), SqlValue::Integer(11)),
                    ("content".to_string(), SqlValue::Text("b".to_string())),
                    ("seq".to_string(), SqlValue::Integer(0)),
                ],
            ],
            conflict_key: None,
        };
        storage.batch_upsert(spec).await.expect("setup rows");

        // 批量更新 seq
        storage
            .batch_update(
                "messages",
                "id",
                &[SqlValue::Integer(10), SqlValue::Integer(11)],
                vec![("seq".to_string(), SqlValue::Integer(99))],
            )
            .await
            .expect("batch_update");

        let r = storage
            .get(GetSpec {
                table: "messages",
                key_col: "id",
                key_val: SqlValue::Integer(10),
            })
            .await
            .expect("get")
            .expect("row");
        let seq = r.iter().find(|(k, _)| k == "seq").unwrap();
        assert!(matches!(&seq.1, SqlValue::Integer(99)));
    }

    #[tokio::test]
    async fn test_monotonic_upsert_max_guard() {
        let storage = in_memory_storage().await;

        // 首次：value=5
        storage
            .monotonic_upsert(MonotonicUpsertSpec {
                scope_key: "ch:1".to_string(),
                value: 5,
            })
            .await
            .expect("first");

        // 推进：value=10（> 5，应写入）
        storage
            .monotonic_upsert(MonotonicUpsertSpec {
                scope_key: "ch:1".to_string(),
                value: 10,
            })
            .await
            .expect("advance");

        // 退行：value=3（< 10，MAX guard 应拒绝，保持 10）
        storage
            .monotonic_upsert(MonotonicUpsertSpec {
                scope_key: "ch:1".to_string(),
                value: 3,
            })
            .await
            .expect("reject smaller");

        let row = storage
            .get(GetSpec {
                table: "_helix_monotonic",
                key_col: "scope_key",
                key_val: SqlValue::Text("ch:1".to_string()),
            })
            .await
            .expect("get monotonic")
            .expect("should exist");
        let val = row.iter().find(|(k, _)| k == "value").unwrap();
        assert!(
            matches!(&val.1, SqlValue::Integer(10)),
            "MAX guard must keep 10, got {:?}",
            val.1
        );
    }

    #[tokio::test]
    async fn test_scan_with_hard_limit() {
        let storage = in_memory_storage().await;
        setup_test_table(&storage).await;

        // 插入 5 行
        for i in 0..5i64 {
            let spec = UpsertSpec {
                table: "messages",
                rows: vec![vec![
                    ("id".to_string(), SqlValue::Integer(100 + i)),
                    ("content".to_string(), SqlValue::Text(format!("msg{i}"))),
                    ("seq".to_string(), SqlValue::Integer(i)),
                ]],
                conflict_key: None,
            };
            storage.batch_upsert(spec).await.expect("insert row");
        }

        let rows = storage
            .scan(ScanSpec {
                table: "messages",
                limit: Some(3),
            })
            .await
            .expect("scan");
        assert_eq!(rows.len(), 3, "scan with limit=3 should return 3 rows");

        let all = storage
            .scan(ScanSpec {
                table: "messages",
                limit: None,
            })
            .await
            .expect("scan all");
        // 5 rows inserted but there might be more from other tests; at least 5
        assert!(
            all.len() >= 5,
            "scan None should return all rows (at least 5)"
        );
    }

    #[tokio::test]
    async fn test_get_not_found() {
        let storage = in_memory_storage().await;
        setup_test_table(&storage).await;

        let row = storage
            .get(GetSpec {
                table: "messages",
                key_col: "id",
                key_val: SqlValue::Integer(9999),
            })
            .await
            .expect("get");
        assert!(row.is_none(), "nonexistent key should return None");
    }
}
