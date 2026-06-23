//! E9b：batch_upsert SQL 形状缓存（HX-C005 去 per-call 分配尾巴）。
//!
//! batch_upsert 的 SQL 文本只由「形状」决定：表名 + 有序列名集 + conflict_key +
//! exclude_from_update。同形状的两次调用产出**逐字相同**的 SQL 串（值绑定走 `?` 参数，
//! 不进文本）。把已建 SQL 按形状 key 缓存，命中即免去 `format!`/`join`/中间 `Vec` 分配。
//! 行为零变更——缓存的是 SQL 文本本身，与原 storage.rs 内联实现逐字等价。

use std::cell::RefCell;
use std::collections::HashMap;

use helix_core::effect::UpsertSpec;

/// batch_upsert SQL 形状键：决定 SQL 文本的全部输入（`Eq + Hash` 用作缓存键）。
///
/// 列名取自 `Row` 的 `String` key（owned），首次构建时 clone 入键；命中后零分配。
#[derive(PartialEq, Eq, Hash)]
struct UpsertShapeKey {
    table: &'static str,
    conflict_key: Option<&'static str>,
    /// 有序列名（顺序影响 INSERT 列序与占位符，须纳入键）。
    col_names: Vec<String>,
    /// 冲突排除列（影响 DO UPDATE SET 集，须纳入键）。
    exclude_from_update: Vec<&'static str>,
}

thread_local! {
    /// 每个 tokio 阻塞线程私有的形状→SQL 缓存。thread-local = 无锁、无跨线程共享；
    /// spawn_blocking 命中后直接复用串。线程池规模有限，形状种类有界 → 缓存不膨胀。
    static UPSERT_SQL_CACHE: RefCell<HashMap<UpsertShapeKey, String>> =
        RefCell::new(HashMap::new());
}

/// 取得（必要时构建并缓存）本 `spec` 形状对应的 batch_upsert SQL 串。
///
/// 调用方须保证 `spec.rows` 非空（storage.rs 已在入口短路空批）。
pub fn upsert_sql_for_shape(spec: &UpsertSpec) -> String {
    let col_names: Vec<String> = spec.rows[0].iter().map(|(k, _)| k.clone()).collect();
    let key = UpsertShapeKey {
        table: spec.table,
        conflict_key: spec.conflict_key,
        col_names,
        exclude_from_update: spec.exclude_from_update.clone(),
    };
    UPSERT_SQL_CACHE.with(|cache| {
        if let Some(sql) = cache.borrow().get(&key) {
            return sql.clone();
        }
        let sql = build_upsert_sql(&key);
        cache.borrow_mut().insert(key, sql.clone());
        sql
    })
}

/// 纯函数：由形状键构建 batch_upsert SQL 文本（与原内联实现逐字等价）。
fn build_upsert_sql(key: &UpsertShapeKey) -> String {
    let cols_sql = key.col_names.join(", ");
    let placeholders = key
        .col_names
        .iter()
        .map(|_| "?")
        .collect::<Vec<_>>()
        .join(", ");
    match key.conflict_key {
        Some(conflict_key) => {
            // ON CONFLICT 更新集排除：conflict_key 自身 + 调用方声明的 exclude_from_update
            // （守卫 / 本地维护列——冲突时保留既有值 = "回退本地"，HX-C005 无需先读后写）。
            let update_cols: Vec<String> = key
                .col_names
                .iter()
                .filter(|c| {
                    c.as_str() != conflict_key && !key.exclude_from_update.contains(&c.as_str())
                })
                .map(|c| format!("{c} = excluded.{c}"))
                .collect();
            if update_cols.is_empty() {
                format!(
                    "INSERT INTO {table} ({cols}) VALUES ({placeholders}) ON CONFLICT({conflict_key}) DO NOTHING",
                    table = key.table,
                    cols = cols_sql,
                )
            } else {
                format!(
                    "INSERT INTO {table} ({cols}) VALUES ({placeholders}) ON CONFLICT({conflict_key}) DO UPDATE SET {updates}",
                    table = key.table,
                    cols = cols_sql,
                    updates = update_cols.join(", "),
                )
            }
        }
        None => format!(
            "INSERT OR REPLACE INTO {table} ({cols}) VALUES ({placeholders})",
            table = key.table,
            cols = cols_sql,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use helix_core::effect::{Row, SqlValue};

    fn row(cols: &[(&str, i64)]) -> Row {
        cols.iter()
            .map(|(k, v)| (k.to_string(), SqlValue::Integer(*v)))
            .collect()
    }

    fn spec(
        table: &'static str,
        cols: &[(&str, i64)],
        conflict_key: Option<&'static str>,
        exclude: Vec<&'static str>,
    ) -> UpsertSpec {
        UpsertSpec {
            table,
            rows: vec![row(cols)],
            conflict_key,
            exclude_from_update: exclude,
        }
    }

    /// 同形状第二次调用复用缓存串：返回值逐字相同（行为零变更 + 命中复用）。
    #[test]
    fn same_shape_reuses_cached_sql() {
        let s1 = spec("message", &[("id", 1), ("content", 0)], Some("id"), vec![]);
        let s2 = spec("message", &[("id", 9), ("content", 0)], Some("id"), vec![]);
        let sql1 = upsert_sql_for_shape(&s1);
        let sql2 = upsert_sql_for_shape(&s2);
        assert_eq!(
            sql1, sql2,
            "同 (table, 列名集, conflict_key, exclude) 形状须产出逐字相同 SQL"
        );
        assert_eq!(
            sql1,
            "INSERT INTO message (id, content) VALUES (?, ?) ON CONFLICT(id) DO UPDATE SET content = excluded.content",
        );
    }

    /// 不同形状各自正确：表名 / conflict / exclude / 无冲突键各产出对应 SQL。
    #[test]
    fn distinct_shapes_each_correct() {
        // 不同表名。
        let other_table = upsert_sql_for_shape(&spec("evt", &[("id", 1)], Some("id"), vec![]));
        assert_eq!(
            other_table, "INSERT INTO evt (id) VALUES (?) ON CONFLICT(id) DO NOTHING",
            "单列即 conflict_key → DO NOTHING（无可更新列）"
        );

        // 无 conflict_key → INSERT OR REPLACE。
        let no_conflict = upsert_sql_for_shape(&spec("kv", &[("k", 1), ("v", 2)], None, vec![]));
        assert_eq!(
            no_conflict,
            "INSERT OR REPLACE INTO kv (k, v) VALUES (?, ?)",
        );

        // exclude_from_update 排除某列（INSERT 仍含该列，DO UPDATE SET 不含）。
        let with_exclude = upsert_sql_for_shape(&spec(
            "ch",
            &[("id", 1), ("name", 0), ("local_flag", 0)],
            Some("id"),
            vec!["local_flag"],
        ));
        assert_eq!(
            with_exclude,
            "INSERT INTO ch (id, name, local_flag) VALUES (?, ?, ?) ON CONFLICT(id) DO UPDATE SET name = excluded.name",
            "exclude 列出现在 INSERT 列集，但被 DO UPDATE SET 排除"
        );
    }
}
