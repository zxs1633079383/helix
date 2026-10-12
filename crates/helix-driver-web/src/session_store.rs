//! Bounded, page-session-only sparse relational store.
//!
//! Capacity is fail-closed: a write either fits completely or returns an error.
//! The store never silently evicts rows, because a partial cache followed by a
//! persisted cursor/coverage proof would let the business layer mistake missing
//! data for a complete local window.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use bytes::Bytes;
use helix_core::effect::{
    BatchDeleteSpec, BatchUpdateSpec, GetSpec, GuardedBumpSpec, MonotonicUpsertSpec, Row, ScanSpec,
    ScopedGetSpec, ScopedGuardedBumpSpec, ScopedMaxSpec, ScopedScanSpec, SortDirection,
    SparseTableSpec, SqlValue, StorageOp, UpsertGuard, UpsertSpec,
};
use helix_core::port_codec::{classify_port_error, rows_to_reply_bytes};
use helix_core::ports::Storage;
use helix_core::tick::{PortOutcome, ReplyBytes};
use helix_core::PortError;
use serde::{Deserialize, Serialize};

#[cfg(not(target_arch = "wasm32"))]
type NowSource = dyn Fn() -> u64 + Send + Sync;
#[cfg(target_arch = "wasm32")]
type NowSource = dyn Fn() -> u64;

/// Explicit limits supplied by the Web platform bootstrap.
///
/// There is deliberately no `Default`: product capacity is a deployment choice,
/// not a value the Rust driver should silently guess.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SessionStoreLimits {
    pub max_rows_per_table: usize,
    pub max_rows_per_scope: usize,
    pub max_scan_rows: usize,
}

impl SessionStoreLimits {
    pub fn validate(self) -> Result<Self, PortError> {
        if self.max_rows_per_table == 0 || self.max_rows_per_scope == 0 || self.max_scan_rows == 0 {
            return Err(storage_error(
                "session store limits must be greater than zero",
            ));
        }
        Ok(self)
    }
}

#[derive(Debug, Clone)]
struct StoredRow {
    values: Row,
    inserted_seq: u64,
}

#[derive(Debug, Clone)]
struct TableState {
    identity_cols: Vec<&'static str>,
    scope_col: Option<&'static str>,
    rows: Vec<StoredRow>,
}

#[derive(Debug, Clone)]
struct StoreState {
    tables: HashMap<&'static str, TableState>,
    next_insert_seq: u64,
    fault: Option<String>,
}

/// A bounded in-memory Storage implementation whose lifetime is one page session.
#[derive(Clone)]
pub struct WebSessionStore {
    state: Arc<Mutex<StoreState>>,
    limits: SessionStoreLimits,
    now_ms: Arc<NowSource>,
}

impl std::fmt::Debug for WebSessionStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("WebSessionStore")
            .field("limits", &self.limits)
            .finish_non_exhaustive()
    }
}

impl WebSessionStore {
    #[cfg(not(target_arch = "wasm32"))]
    pub fn new(
        specs: &[SparseTableSpec],
        limits: SessionStoreLimits,
        now_ms: impl Fn() -> u64 + Send + Sync + 'static,
    ) -> Result<Self, PortError> {
        Self::from_parts(specs, limits.validate()?, Arc::new(now_ms))
    }

    #[cfg(target_arch = "wasm32")]
    pub fn new(
        specs: &[SparseTableSpec],
        limits: SessionStoreLimits,
        now_ms: impl Fn() -> u64 + 'static,
    ) -> Result<Self, PortError> {
        Self::from_parts(specs, limits.validate()?, Arc::new(now_ms))
    }

    fn from_parts(
        specs: &[SparseTableSpec],
        limits: SessionStoreLimits,
        now_ms: Arc<NowSource>,
    ) -> Result<Self, PortError> {
        let mut tables = HashMap::with_capacity(specs.len());
        for spec in specs {
            if spec.identity_cols.is_empty() {
                return Err(storage_error(format!(
                    "table {} must declare at least one identity column",
                    spec.table
                )));
            }
            if tables
                .insert(
                    spec.table,
                    TableState {
                        identity_cols: spec.identity_cols.to_vec(),
                        scope_col: spec.scope_col,
                        rows: Vec::new(),
                    },
                )
                .is_some()
            {
                return Err(storage_error(format!(
                    "duplicate sparse table spec: {}",
                    spec.table
                )));
            }
        }

        Ok(Self {
            state: Arc::new(Mutex::new(StoreState {
                tables,
                next_insert_seq: 0,
                fault: None,
            })),
            limits,
            now_ms,
        })
    }

    /// Execute a `StorageOp` batch with the same fail-fast/read-result contract
    /// as the host driver. Earlier successful ops are not rolled back when a
    /// later, distinct op fails.
    pub fn execute_ops(&self, ops: &[StorageOp]) -> PortOutcome {
        if let Err(error) = self.ensure_healthy() {
            return PortOutcome::Err(classify_port_error(error));
        }
        let mut last_read = Bytes::new();
        for op in ops {
            let result = match op {
                StorageOp::BatchUpsert(spec) => self.batch_upsert_sync(spec.clone()),
                StorageOp::BatchUpdate(spec) => self.batch_update_sync(spec.clone()),
                StorageOp::MonotonicUpsert(spec) => self.monotonic_upsert_sync(spec.clone()),
                StorageOp::GuardedBump(spec) => self.guarded_bump_sync(spec.clone()),
                StorageOp::ScopedGuardedBump(spec) => self.scoped_guarded_bump_sync(spec.clone()),
                StorageOp::Get(spec) => self.get_sync(spec.clone()).map(|row| {
                    last_read = row
                        .map(|row| rows_to_reply_bytes(&[row]))
                        .unwrap_or_else(|| rows_to_reply_bytes(&[]));
                }),
                StorageOp::ScopedGet(spec) => self.scoped_get_sync(spec.clone()).map(|row| {
                    last_read = row
                        .map(|row| rows_to_reply_bytes(&[row]))
                        .unwrap_or_else(|| rows_to_reply_bytes(&[]));
                }),
                StorageOp::ScopedMax(spec) => self.scoped_max_sync(spec.clone()).map(|row| {
                    last_read = row
                        .map(|row| rows_to_reply_bytes(&[row]))
                        .unwrap_or_else(|| rows_to_reply_bytes(&[]));
                }),
                StorageOp::ScopedScan(spec) => self.scoped_scan_sync(spec.clone()).map(|rows| {
                    last_read = rows_to_reply_bytes(&rows);
                }),
                StorageOp::Scan(spec) => self.scan_sync(spec.clone()).map(|rows| {
                    last_read = rows_to_reply_bytes(&rows);
                }),
                StorageOp::BatchDelete(spec) => self.batch_delete_sync(spec.clone()),
            };
            if let Err(error) = result {
                self.latch_fault(&error);
                return PortOutcome::Err(classify_port_error(error));
            }
        }
        PortOutcome::Ok(ReplyBytes(last_read))
    }

    /// Execute one shell step's fire-and-forget writes as a single session
    /// transaction. A later failure restores the pre-step snapshot, then
    /// latches the store unavailable so local reads fall back to remote.
    pub fn execute_ops_atomic(&self, ops: &[StorageOp]) -> PortOutcome {
        let checkpoint = match self.lock() {
            Ok(state) => state.clone(),
            Err(error) => return PortOutcome::Err(classify_port_error(error)),
        };
        let outcome = self.execute_ops(ops);
        if matches!(&outcome, PortOutcome::Err(_)) {
            if let Ok(mut state) = self.state.lock() {
                let fault = state.fault.clone();
                *state = checkpoint;
                state.fault = fault;
            }
        }
        outcome
    }

    /// Execute the write-only contract behind `Effect::PersistAtomic`.
    ///
    /// Keep `execute_ops_atomic` unchanged for legacy `PersistFire`; the correlated primitive
    /// rejects reads explicitly so a PortReply means one all-write transaction committed.
    pub fn execute_atomic_write(&self, ops: &[StorageOp]) -> PortOutcome {
        if ops.iter().any(|op| {
            matches!(
                op,
                StorageOp::Get(_)
                    | StorageOp::ScopedGet(_)
                    | StorageOp::ScopedMax(_)
                    | StorageOp::ScopedScan(_)
                    | StorageOp::Scan(_)
            )
        }) {
            return PortOutcome::Err(classify_port_error(storage_error(
                "atomic storage write does not permit read operations",
            )));
        }
        self.execute_ops_atomic(ops)
    }

    pub fn row_count(&self, table: &'static str) -> Result<usize, PortError> {
        let state = self.lock()?;
        Ok(self.table(&state, table)?.rows.len())
    }

    pub fn is_healthy(&self) -> bool {
        self.state
            .lock()
            .map(|state| state.fault.is_none())
            .unwrap_or(false)
    }

    /// Apply a bounded sparse upsert, checking an optional single-row CAS guard before merging.
    fn batch_upsert_sync(&self, spec: UpsertSpec) -> Result<(), PortError> {
        if let Some(column) = spec.version_column {
            if spec.conflict_key.is_none()
                || spec
                    .rows
                    .iter()
                    .any(|row| matches!(row_value(row, column), None | Some(SqlValue::Null)))
            {
                return Err(storage_error(
                    "versioned upsert requires a conflict key and non-null version in every row",
                ));
            }
        }
        if spec.update_guard.is_some() {
            if spec.conflict_key.is_none() {
                return Err(storage_error("upsert update guard requires a conflict key"));
            }
            if spec.rows.len() != 1 {
                return Err(storage_error(
                    "upsert update guard requires a single-row batch",
                ));
            }
        }
        if spec.rows.is_empty() {
            return Ok(());
        }
        if spec.rows.len() > self.limits.max_rows_per_table {
            return Err(storage_error(format!(
                "table {} write batch exceeds row capacity {}",
                spec.table, self.limits.max_rows_per_table
            )));
        }
        validate_row_shapes(&spec.rows)?;

        let mut state = self.lock()?;
        let table = self.table(&state, spec.table)?;
        let replace_on_conflict = spec.conflict_key.is_none();
        let conflict_cols = match spec.conflict_key {
            Some(key) => parse_columns(key)?,
            None => table.identity_cols.clone(),
        };
        if conflict_cols != table.identity_cols {
            return Err(storage_error(format!(
                "table {} conflict key must match its declared sparse identity",
                spec.table
            )));
        }
        for incoming in &spec.rows {
            require_columns(incoming, &conflict_cols, spec.table)?;
        }
        preflight_upsert_capacity(
            spec.table,
            table,
            &spec.rows,
            replace_on_conflict,
            &spec.exclude_from_update,
            self.limits,
        )?;

        let mut table = table.clone();
        let mut next_insert_seq = state.next_insert_seq;

        for incoming in spec.rows {
            if let Some(index) = table
                .rows
                .iter()
                .position(|stored| rows_match(&stored.values, &incoming, &conflict_cols))
            {
                if spec.version_column.is_some_and(|column| {
                    let current = row_value(&table.rows[index].values, column);
                    !matches!(current, None | Some(SqlValue::Null))
                        && sql_value_cmp(row_value(&incoming, column), current) == Ordering::Less
                }) {
                    if spec.update_guard.is_some() {
                        return Err(PortError::StorageConflict);
                    }
                    continue;
                }
                if let Some(guard) = spec.update_guard.as_ref() {
                    if !upsert_guard_matches(&table.rows[index].values, guard) {
                        return Err(PortError::StorageConflict);
                    }
                }
                if replace_on_conflict {
                    table.rows[index] = StoredRow {
                        values: incoming,
                        inserted_seq: next_insert_seq,
                    };
                    next_insert_seq = next_insert_seq.saturating_add(1);
                } else {
                    merge_upsert_row(
                        &mut table.rows[index].values,
                        incoming,
                        &conflict_cols,
                        &spec.exclude_from_update,
                    );
                }
            } else {
                table.rows.push(StoredRow {
                    values: incoming,
                    inserted_seq: next_insert_seq,
                });
                next_insert_seq = next_insert_seq.saturating_add(1);
            }
        }

        self.validate_table(spec.table, &table)?;
        *self.table_mut(&mut state, spec.table)? = table;
        state.next_insert_seq = next_insert_seq;
        Ok(())
    }

    fn batch_update_sync(&self, spec: BatchUpdateSpec) -> Result<(), PortError> {
        if spec.key_vals.is_empty() || spec.patch.is_empty() {
            return Ok(());
        }
        let mut state = self.lock()?;
        let mut table = self.table(&state, spec.table)?.clone();
        reject_identity_writes(
            spec.table,
            &table,
            spec.patch.iter().map(|(column, _)| column.as_str()),
        )?;
        for stored in &mut table.rows {
            if row_value(&stored.values, spec.key_col)
                .is_some_and(|value| spec.key_vals.iter().any(|key| sql_value_eq(value, key)))
            {
                merge_patch(&mut stored.values, &spec.patch);
            }
        }
        self.validate_table(spec.table, &table)?;
        *self.table_mut(&mut state, spec.table)? = table;
        Ok(())
    }

    fn monotonic_upsert_sync(&self, spec: MonotonicUpsertSpec) -> Result<(), PortError> {
        let now_ms = spec.touch_col.map(|_| (self.now_ms)() as i64);
        let mut state = self.lock()?;
        let mut table = self.table(&state, spec.table)?.clone();
        if table.identity_cols.as_slice() != [spec.key_col] {
            return Err(storage_error(format!(
                "table {} monotonic key must match its declared sparse identity",
                spec.table
            )));
        }
        reject_identity_writes(
            spec.table,
            &table,
            std::iter::once(spec.value_col).chain(spec.touch_col),
        )?;
        let key_value = SqlValue::Text(spec.scope_key.clone());
        if let Some(stored) = table.rows.iter_mut().find(|stored| {
            row_value(&stored.values, spec.key_col)
                .is_some_and(|value| sql_value_eq(value, &key_value))
        }) {
            let current = integer_or_zero(row_value(&stored.values, spec.value_col))?;
            if spec.value > current {
                set_row_value(
                    &mut stored.values,
                    spec.value_col,
                    SqlValue::Integer(spec.value),
                );
            }
            // Match the delivered host implementation: touch is refreshed on
            // every conflict, while the monotonic value itself never regresses.
            if let Some(touch_col) = spec.touch_col {
                set_row_value(
                    &mut stored.values,
                    touch_col,
                    SqlValue::Integer(now_ms.unwrap_or_default()),
                );
            }
        } else {
            let mut row = vec![
                (spec.key_col.to_string(), key_value),
                (spec.value_col.to_string(), SqlValue::Integer(spec.value)),
            ];
            if let Some(touch_col) = spec.touch_col {
                row.push((
                    touch_col.to_string(),
                    SqlValue::Integer(now_ms.unwrap_or_default()),
                ));
            }
            table.rows.push(StoredRow {
                values: row,
                inserted_seq: state.next_insert_seq,
            });
            state.next_insert_seq = state.next_insert_seq.saturating_add(1);
        }
        self.validate_table(spec.table, &table)?;
        *self.table_mut(&mut state, spec.table)? = table;
        Ok(())
    }

    fn guarded_bump_sync(&self, spec: GuardedBumpSpec) -> Result<(), PortError> {
        let mut state = self.lock()?;
        let mut table = self.table(&state, spec.table)?.clone();
        reject_identity_writes(
            spec.table,
            &table,
            std::iter::once(spec.bump_col)
                .chain(spec.extra_bumps.iter().map(|(column, _)| *column))
                .chain(spec.set_cols.iter().map(|(column, _)| column.as_str())),
        )?;
        let Some(stored) = table.rows.iter_mut().find(|stored| {
            row_value(&stored.values, spec.key_col)
                .is_some_and(|value| sql_value_eq(value, &spec.key_val))
        }) else {
            return Ok(());
        };

        let current_guard = integer_or_zero(row_value(&stored.values, spec.guard_col))?;
        if spec.guard_val <= current_guard {
            return Ok(());
        }

        bump_column(&mut stored.values, spec.bump_col, spec.bump_delta)?;
        for (column, delta) in spec.extra_bumps {
            bump_column(&mut stored.values, column, delta)?;
        }
        merge_patch(&mut stored.values, &spec.set_cols);
        self.validate_table(spec.table, &table)?;
        *self.table_mut(&mut state, spec.table)? = table;
        Ok(())
    }

    /// 在复合 identity 行内执行前进守卫计数更新。
    fn scoped_guarded_bump_sync(&self, spec: ScopedGuardedBumpSpec) -> Result<(), PortError> {
        let mut state = self.lock()?;
        let mut table = self.table(&state, spec.table)?.clone();
        reject_identity_writes(
            spec.table,
            &table,
            std::iter::once(spec.bump_col)
                .chain(spec.set_cols.iter().map(|(column, _)| column.as_str())),
        )?;
        let Some(stored) = table.rows.iter_mut().find(|stored| {
            row_value(&stored.values, spec.scope_col)
                .is_some_and(|value| sql_value_eq(value, &spec.scope_val))
                && row_value(&stored.values, spec.key_col)
                    .is_some_and(|value| sql_value_eq(value, &spec.key_val))
        }) else {
            return Ok(());
        };
        if let Some((column, expected)) = spec.additional_guard.as_ref() {
            if !row_value(&stored.values, column).is_some_and(|value| sql_value_eq(value, expected))
            {
                return Ok(());
            }
        }
        let current_guard = integer_or_zero(row_value(&stored.values, spec.guard_col))?;
        if !match spec.comparison {
            helix_core::effect::GuardComparison::Greater => spec.guard_val > current_guard,
            helix_core::effect::GuardComparison::Equal => spec.guard_val == current_guard,
        } {
            return Ok(());
        }
        bump_column(&mut stored.values, spec.bump_col, spec.bump_delta)?;
        merge_patch(&mut stored.values, &spec.set_cols);
        self.validate_table(spec.table, &table)?;
        *self.table_mut(&mut state, spec.table)? = table;
        Ok(())
    }

    fn get_sync(&self, spec: GetSpec) -> Result<Option<Row>, PortError> {
        let state = self.lock()?;
        let table = self.table(&state, spec.table)?;
        Ok(table
            .rows
            .iter()
            .find(|stored| {
                row_value(&stored.values, spec.key_col)
                    .is_some_and(|value| sql_value_eq(value, &spec.key_val))
            })
            .map(|stored| stored.values.clone()))
    }

    /// 以两个 identity 列读取复合主键行。
    fn scoped_get_sync(&self, spec: ScopedGetSpec) -> Result<Option<Row>, PortError> {
        let state = self.lock()?;
        let table = self.table(&state, spec.table)?;
        Ok(table
            .rows
            .iter()
            .find(|stored| {
                row_value(&stored.values, spec.scope_col)
                    .is_some_and(|value| sql_value_eq(value, &spec.scope_val))
                    && row_value(&stored.values, spec.key_col)
                        .is_some_and(|value| sql_value_eq(value, &spec.key_val))
            })
            .map(|stored| stored.values.clone()))
    }

    /// 在作用域集合内求指定列最大值；空集合不触碰任何表行。
    fn scoped_max_sync(&self, spec: ScopedMaxSpec) -> Result<Option<Row>, PortError> {
        if spec.scope_values.is_empty() {
            return Ok(None);
        }
        let mut scoped_values: Vec<&SqlValue> = spec.scope_values.iter().collect();
        scoped_values.sort_by(|left, right| sql_value_cmp(Some(*left), Some(*right)));
        let state = self.lock()?;
        let table = self.table(&state, spec.table)?;
        let maximum = table
            .rows
            .iter()
            .filter(|stored| {
                row_value(&stored.values, spec.scope_col).is_some_and(|scope| {
                    scoped_values
                        .binary_search_by(|value| sql_value_cmp(Some(*value), Some(scope)))
                        .is_ok()
                })
            })
            .filter_map(|stored| row_value(&stored.values, spec.value_col))
            .filter(|value| !matches!(value, SqlValue::Null))
            .max_by(|left, right| sql_value_cmp(Some(*left), Some(*right)));
        Ok(maximum.map(|value| vec![(spec.result_alias.to_string(), value.clone())]))
    }

    /// 在作用域集合内读取有界多行；超过显式 limit 时 fail-closed，不静默截断。
    fn scoped_scan_sync(&self, spec: ScopedScanSpec) -> Result<Vec<Row>, PortError> {
        const MAX_SCOPED_SCAN_LIMIT: usize = 10_000;
        if spec.scope_values.is_empty() {
            return Ok(Vec::new());
        }
        if spec.limit > MAX_SCOPED_SCAN_LIMIT {
            return Err(storage_error(format!(
                "scoped scan limit exceeds driver maximum {MAX_SCOPED_SCAN_LIMIT}"
            )));
        }
        let mut scoped_values: Vec<&SqlValue> = spec.scope_values.iter().collect();
        scoped_values.sort_by(|left, right| sql_value_cmp(Some(*left), Some(*right)));
        scoped_values
            .dedup_by(|left, right| sql_value_cmp(Some(*left), Some(*right)) == Ordering::Equal);
        let state = self.lock()?;
        let table = self.table(&state, spec.table)?;
        let mut rows = Vec::new();
        for stored in &table.rows {
            let Some(scope) = row_value(&stored.values, spec.scope_col) else {
                continue;
            };
            if scoped_values
                .binary_search_by(|value| sql_value_cmp(Some(*value), Some(scope)))
                .is_ok()
            {
                rows.push(&stored.values);
                if rows.len() > spec.limit {
                    return Err(storage_error("scoped scan result exceeds explicit limit"));
                }
            }
        }
        Ok(rows.into_iter().cloned().collect())
    }

    fn scan_sync(&self, spec: ScanSpec) -> Result<Vec<Row>, PortError> {
        let state = self.lock()?;
        let table = self.table(&state, spec.table)?;
        let mut rows: Vec<&StoredRow> = table
            .rows
            .iter()
            .filter(|stored| match &spec.filter {
                Some((column, expected)) => row_value(&stored.values, column)
                    .is_some_and(|value| sql_value_eq(value, expected)),
                None => true,
            })
            .collect();

        if !spec.order_by.is_empty() {
            rows.sort_by(|left, right| {
                for order in spec.order_by {
                    let ordering = match order.direction {
                        SortDirection::Asc => sql_value_cmp(
                            row_value(&left.values, order.column),
                            row_value(&right.values, order.column),
                        ),
                        SortDirection::Desc => sql_value_cmp(
                            row_value(&right.values, order.column),
                            row_value(&left.values, order.column),
                        ),
                    };
                    if ordering != Ordering::Equal {
                        return ordering;
                    }
                }
                left.inserted_seq.cmp(&right.inserted_seq)
            });
        }

        let requested = spec.limit.map(|limit| limit as usize).unwrap_or(usize::MAX);
        let limit = requested.min(self.limits.max_scan_rows);
        Ok(rows
            .into_iter()
            .take(limit)
            .map(|stored| stored.values.clone())
            .collect())
    }

    fn batch_delete_sync(&self, spec: BatchDeleteSpec) -> Result<(), PortError> {
        if spec.key_vals.is_empty() {
            return Ok(());
        }
        let mut state = self.lock()?;
        let table = self.table_mut(&mut state, spec.table)?;
        table.rows.retain(|stored| {
            let in_scope = row_value(&stored.values, spec.scope_col)
                .is_some_and(|value| sql_value_eq(value, &spec.scope_val));
            let key_matches = row_value(&stored.values, spec.key_col)
                .is_some_and(|value| spec.key_vals.iter().any(|key| sql_value_eq(value, key)));
            !(in_scope && key_matches)
        });
        Ok(())
    }

    fn validate_table(
        &self,
        table_name: &'static str,
        table: &TableState,
    ) -> Result<(), PortError> {
        for stored in &table.rows {
            require_columns(&stored.values, &table.identity_cols, table_name)?;
        }
        if table.rows.len() > self.limits.max_rows_per_table {
            return Err(storage_error(format!(
                "table {table_name} capacity {} exceeded",
                self.limits.max_rows_per_table
            )));
        }
        let Some(scope_col) = table.scope_col else {
            return Ok(());
        };
        let mut text_scopes: HashMap<&str, usize> = HashMap::new();
        let mut other_scopes: Vec<(&SqlValue, usize)> = Vec::new();
        for stored in &table.rows {
            let scope = row_value(&stored.values, scope_col).ok_or_else(|| {
                storage_error(format!(
                    "table {table_name} row is missing scope column {scope_col}"
                ))
            })?;
            let count = match scope {
                SqlValue::Text(value) => text_scopes.entry(value).or_default(),
                _ => {
                    if let Some(index) = other_scopes
                        .iter()
                        .position(|(existing, _)| sql_value_eq(existing, scope))
                    {
                        &mut other_scopes[index].1
                    } else {
                        other_scopes.push((scope, 1));
                        continue;
                    }
                }
            };
            *count += 1;
            if *count > self.limits.max_rows_per_scope {
                return Err(storage_error(format!(
                    "table {table_name} scope capacity {} exceeded",
                    self.limits.max_rows_per_scope
                )));
            }
        }
        Ok(())
    }

    fn lock(&self) -> Result<MutexGuard<'_, StoreState>, PortError> {
        let state = self
            .state
            .lock()
            .map_err(|_| storage_error("session store lock poisoned"))?;
        if let Some(fault) = &state.fault {
            return Err(storage_error(format!(
                "session store disabled after prior failure: {fault}"
            )));
        }
        Ok(state)
    }

    fn ensure_healthy(&self) -> Result<(), PortError> {
        drop(self.lock()?);
        Ok(())
    }

    /// Only backend failures disable the session; optimistic conflicts are retryable.
    fn latch_fault(&self, error: &PortError) {
        if matches!(error, PortError::StorageConflict) {
            return;
        }
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        if state.fault.is_none() {
            state.fault = Some(error.to_string());
        }
    }

    fn table<'a>(
        &self,
        state: &'a StoreState,
        table: &'static str,
    ) -> Result<&'a TableState, PortError> {
        state
            .tables
            .get(table)
            .ok_or_else(|| storage_error(format!("unregistered session table: {table}")))
    }

    fn table_mut<'a>(
        &self,
        state: &'a mut StoreState,
        table: &'static str,
    ) -> Result<&'a mut TableState, PortError> {
        state
            .tables
            .get_mut(table)
            .ok_or_else(|| storage_error(format!("unregistered session table: {table}")))
    }
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl Storage for WebSessionStore {
    async fn batch_upsert(&self, spec: UpsertSpec) -> Result<(), PortError> {
        self.ensure_healthy()?;
        let result = self.batch_upsert_sync(spec);
        if let Err(error) = &result {
            self.latch_fault(error);
        }
        result
    }

    async fn batch_update(&self, spec: BatchUpdateSpec) -> Result<(), PortError> {
        self.ensure_healthy()?;
        let result = self.batch_update_sync(spec);
        if let Err(error) = &result {
            self.latch_fault(error);
        }
        result
    }

    async fn monotonic_upsert(&self, spec: MonotonicUpsertSpec) -> Result<(), PortError> {
        self.ensure_healthy()?;
        let result = self.monotonic_upsert_sync(spec);
        if let Err(error) = &result {
            self.latch_fault(error);
        }
        result
    }

    async fn guarded_bump(&self, spec: GuardedBumpSpec) -> Result<(), PortError> {
        self.ensure_healthy()?;
        let result = self.guarded_bump_sync(spec);
        if let Err(error) = &result {
            self.latch_fault(error);
        }
        result
    }

    /// 兑现复合作用域守卫更新并在错误时锁闭 session store。
    async fn scoped_guarded_bump(&self, spec: ScopedGuardedBumpSpec) -> Result<(), PortError> {
        self.ensure_healthy()?;
        let result = self.scoped_guarded_bump_sync(spec);
        if let Err(error) = &result {
            self.latch_fault(error);
        }
        result
    }

    async fn get(&self, spec: GetSpec) -> Result<Option<Row>, PortError> {
        self.get_sync(spec)
    }

    /// 读取复合作用域内唯一行。
    async fn scoped_get(&self, spec: ScopedGetSpec) -> Result<Option<Row>, PortError> {
        self.scoped_get_sync(spec)
    }

    /// 读取复合作用域集合内的最大值。
    async fn scoped_max(&self, spec: ScopedMaxSpec) -> Result<Option<Row>, PortError> {
        self.scoped_max_sync(spec)
    }

    async fn scoped_scan(&self, spec: ScopedScanSpec) -> Result<Vec<Row>, PortError> {
        self.scoped_scan_sync(spec)
    }

    async fn scan(&self, spec: ScanSpec) -> Result<Vec<Row>, PortError> {
        self.scan_sync(spec)
    }

    async fn batch_delete(&self, spec: BatchDeleteSpec) -> Result<(), PortError> {
        self.ensure_healthy()?;
        let result = self.batch_delete_sync(spec);
        if let Err(error) = &result {
            self.latch_fault(error);
        }
        result
    }
}

fn storage_error(message: impl Into<String>) -> PortError {
    PortError::Storage(message.into())
}

fn parse_columns(columns: &'static str) -> Result<Vec<&'static str>, PortError> {
    let parsed: Vec<&str> = columns
        .split(',')
        .map(str::trim)
        .filter(|column| !column.is_empty())
        .collect();
    if parsed.is_empty() {
        return Err(storage_error("empty conflict key"));
    }
    Ok(parsed)
}

fn validate_row_shapes(rows: &[Row]) -> Result<(), PortError> {
    let expected: Vec<&str> = rows[0].iter().map(|(column, _)| column.as_str()).collect();
    if expected
        .iter()
        .enumerate()
        .any(|(index, column)| expected[index + 1..].contains(column))
    {
        return Err(storage_error(
            "batch upsert rows must not contain duplicate columns",
        ));
    }
    if rows.iter().skip(1).any(|row| {
        row.iter()
            .map(|(column, _)| column.as_str())
            .collect::<Vec<_>>()
            != expected
    }) {
        return Err(storage_error(
            "batch upsert rows must have identical column order",
        ));
    }
    Ok(())
}

fn preflight_upsert_capacity<'a>(
    table_name: &'static str,
    table: &TableState,
    rows: &'a [Row],
    replace_on_conflict: bool,
    exclude_from_update: &[&str],
    limits: SessionStoreLimits,
) -> Result<(), PortError> {
    struct IncomingIdentity<'a> {
        first: &'a Row,
        last: &'a Row,
    }

    // Repeated identities are applied in input order. A new merge-upsert first
    // inserts the whole row, so an excluded scope comes from the first row;
    // replacement or an updatable scope ends with the last row. Keep both
    // endpoints to mirror that behavior without cloning payload rows.
    let mut incoming_by_identity: Vec<IncomingIdentity<'a>> = Vec::with_capacity(rows.len());
    for row in rows {
        if let Some(incoming) = incoming_by_identity
            .iter_mut()
            .find(|incoming| rows_match(incoming.first, row, &table.identity_cols))
        {
            incoming.last = row;
        } else {
            incoming_by_identity.push(IncomingIdentity {
                first: row,
                last: row,
            });
        }
    }

    let new_rows = incoming_by_identity
        .iter()
        .filter(|incoming| {
            !table
                .rows
                .iter()
                .any(|stored| rows_match(&stored.values, incoming.first, &table.identity_cols))
        })
        .count();
    let final_rows = table.rows.len().saturating_add(new_rows);
    if final_rows > limits.max_rows_per_table {
        return Err(storage_error(format!(
            "table {table_name} capacity {} exceeded",
            limits.max_rows_per_table
        )));
    }

    let Some(scope_col) = table.scope_col else {
        return Ok(());
    };
    let mut scopes: Vec<(SqlValue, usize)> = Vec::new();
    let mut add_scope = |scope: &SqlValue| -> Result<(), PortError> {
        if let Some((_, count)) = scopes
            .iter_mut()
            .find(|(current, _)| sql_value_eq(current, scope))
        {
            *count += 1;
            if *count > limits.max_rows_per_scope {
                return Err(storage_error(format!(
                    "table {table_name} scope capacity {} exceeded",
                    limits.max_rows_per_scope
                )));
            }
        } else {
            scopes.push((scope.clone(), 1));
        }
        Ok(())
    };

    for stored in &table.rows {
        let incoming = incoming_by_identity
            .iter()
            .find(|incoming| rows_match(&stored.values, incoming.first, &table.identity_cols));
        let existing_scope = row_value(&stored.values, scope_col);
        let scope = match incoming {
            Some(incoming) if replace_on_conflict => row_value(incoming.last, scope_col),
            Some(incoming) if !exclude_from_update.contains(&scope_col) => {
                row_value(incoming.last, scope_col).or(existing_scope)
            }
            _ => existing_scope,
        }
        .ok_or_else(|| {
            storage_error(format!(
                "table {table_name} row is missing scope column {scope_col}"
            ))
        })?;
        add_scope(scope)?;
    }
    for incoming in incoming_by_identity {
        let already_exists = table
            .rows
            .iter()
            .any(|stored| rows_match(&stored.values, incoming.first, &table.identity_cols));
        if already_exists {
            continue;
        }
        let final_row = if replace_on_conflict || !exclude_from_update.contains(&scope_col) {
            incoming.last
        } else {
            incoming.first
        };
        let scope = row_value(final_row, scope_col).ok_or_else(|| {
            storage_error(format!(
                "table {table_name} row is missing scope column {scope_col}"
            ))
        })?;
        add_scope(scope)?;
    }
    Ok(())
}

fn require_columns(row: &Row, columns: &[&str], table: &'static str) -> Result<(), PortError> {
    if let Some(column) = columns
        .iter()
        .find(|column| row_value(row, column).is_none())
    {
        return Err(storage_error(format!(
            "table {table} row is missing required column {column}"
        )));
    }
    Ok(())
}

fn reject_identity_writes<'a>(
    table_name: &'static str,
    table: &TableState,
    columns: impl IntoIterator<Item = &'a str>,
) -> Result<(), PortError> {
    if let Some(column) = columns
        .into_iter()
        .find(|column| table.identity_cols.contains(column))
    {
        return Err(storage_error(format!(
            "table {table_name} sparse identity column {column} is immutable"
        )));
    }
    Ok(())
}

fn rows_match(existing: &Row, incoming: &Row, columns: &[&str]) -> bool {
    columns.iter().all(|column| {
        row_value(existing, column)
            .zip(row_value(incoming, column))
            .is_some_and(|(left, right)| sql_value_eq(left, right))
    })
}

fn merge_upsert_row(
    existing: &mut Row,
    incoming: Row,
    conflict_cols: &[&str],
    exclude_from_update: &[&str],
) {
    for (column, value) in incoming {
        if conflict_cols.contains(&column.as_str())
            || exclude_from_update.contains(&column.as_str())
        {
            continue;
        }
        set_row_value(existing, &column, value);
    }
}

fn merge_patch(row: &mut Row, patch: &Row) {
    for (column, value) in patch {
        set_row_value(row, column, value.clone());
    }
}

fn set_row_value(row: &mut Row, column: &str, value: SqlValue) {
    if let Some((_, current)) = row.iter_mut().find(|(name, _)| name == column) {
        *current = value;
    } else {
        row.push((column.to_string(), value));
    }
}

fn row_value<'a>(row: &'a Row, column: &str) -> Option<&'a SqlValue> {
    row.iter()
        .find(|(name, _)| name == column)
        .map(|(_, value)| value)
}

fn integer_or_zero(value: Option<&SqlValue>) -> Result<i64, PortError> {
    match value {
        Some(SqlValue::Integer(value)) => Ok(*value),
        Some(SqlValue::Null) | None => Ok(0),
        Some(other) => Err(storage_error(format!(
            "expected integer column, got {other:?}"
        ))),
    }
}

fn bump_column(row: &mut Row, column: &str, delta: i64) -> Result<(), PortError> {
    let current = integer_or_zero(row_value(row, column))?;
    set_row_value(
        row,
        column,
        SqlValue::Integer(current.saturating_add(delta)),
    );
    Ok(())
}

fn sql_value_eq(left: &SqlValue, right: &SqlValue) -> bool {
    match (left, right) {
        (SqlValue::Text(left), SqlValue::Text(right)) => left == right,
        (SqlValue::Integer(left), SqlValue::Integer(right)) => left == right,
        (SqlValue::Real(left), SqlValue::Real(right)) => left.to_bits() == right.to_bits(),
        (SqlValue::Integer(left), SqlValue::Real(right)) => (*left as f64) == *right,
        (SqlValue::Real(left), SqlValue::Integer(right)) => *left == (*right as f64),
        (SqlValue::Blob(left), SqlValue::Blob(right)) => left == right,
        (SqlValue::Null, SqlValue::Null) => true,
        _ => false,
    }
}

/// Apply the same null-aware equality as SQLite `IS` to a sparse row column.
fn upsert_guard_matches(row: &Row, guard: &UpsertGuard) -> bool {
    row_value(row, guard.column).map_or(matches!(&guard.expected, &SqlValue::Null), |actual| {
        sql_is_eq(actual, &guard.expected)
    })
}

/// Match SQLite `IS` numeric behavior, including equal real values with distinct zero bits.
fn sql_is_eq(left: &SqlValue, right: &SqlValue) -> bool {
    match (left, right) {
        (SqlValue::Real(left), SqlValue::Real(right)) => left == right,
        _ => sql_value_eq(left, right),
    }
}

fn sql_value_cmp(left: Option<&SqlValue>, right: Option<&SqlValue>) -> Ordering {
    match (left, right) {
        (None | Some(SqlValue::Null), None | Some(SqlValue::Null)) => Ordering::Equal,
        (None | Some(SqlValue::Null), _) => Ordering::Less,
        (_, None | Some(SqlValue::Null)) => Ordering::Greater,
        (Some(SqlValue::Integer(left)), Some(SqlValue::Integer(right))) => left.cmp(right),
        (Some(SqlValue::Real(left)), Some(SqlValue::Real(right))) => {
            left.partial_cmp(right).unwrap_or(Ordering::Equal)
        }
        (Some(SqlValue::Integer(left)), Some(SqlValue::Real(right))) => {
            (*left as f64).partial_cmp(right).unwrap_or(Ordering::Equal)
        }
        (Some(SqlValue::Real(left)), Some(SqlValue::Integer(right))) => left
            .partial_cmp(&(*right as f64))
            .unwrap_or(Ordering::Equal),
        (Some(SqlValue::Text(left)), Some(SqlValue::Text(right))) => left.cmp(right),
        (Some(SqlValue::Blob(left)), Some(SqlValue::Blob(right))) => left.cmp(right),
        (Some(left), Some(right)) => sql_type_rank(left).cmp(&sql_type_rank(right)),
    }
}

fn sql_type_rank(value: &SqlValue) -> u8 {
    match value {
        SqlValue::Null => 0,
        SqlValue::Integer(_) | SqlValue::Real(_) => 1,
        SqlValue::Text(_) => 2,
        SqlValue::Blob(_) => 3,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

    use helix_core::effect::{MonotonicUpsertSpec, ScanOrder};

    use super::*;

    const CREATE_AT_DESC: &[ScanOrder] = &[ScanOrder::desc("create_at")];

    /// Both sequence and extra equality guards must hold on the Web storage path.
    #[test]
    fn scoped_guarded_bump_requires_all_guards() {
        let store = new_store(limits());
        for (viewer, revision) in [("legacy", 0), ("versioned", 8)] {
            store
                .batch_upsert_sync(UpsertSpec {
                    table: "channel_member",
                    rows: vec![vec![
                        ("channel_id".into(), text("channel-a")),
                        ("user_id".into(), text(viewer)),
                        ("projection_revision".into(), SqlValue::Integer(revision)),
                        ("last_unread_event_seq".into(), SqlValue::Integer(1)),
                        ("last_post_at".into(), SqlValue::Integer(0)),
                        ("unread_count".into(), SqlValue::Integer(4)),
                    ]],
                    conflict_key: Some("channel_id,user_id"),
                    exclude_from_update: vec![],
                    version_column: None,
                    update_guard: None,
                })
                .unwrap();
            store
                .scoped_guarded_bump_sync(ScopedGuardedBumpSpec {
                    comparison: helix_core::effect::GuardComparison::Greater,
                    table: "channel_member",
                    scope_col: "channel_id",
                    scope_val: text("channel-a"),
                    key_col: "user_id",
                    key_val: text(viewer),
                    bump_col: "last_post_at",
                    bump_delta: 0,
                    set_cols: vec![("unread_count".into(), SqlValue::Integer(9))],
                    guard_col: "last_unread_event_seq",
                    guard_val: 2,
                    additional_guard: Some(("projection_revision", SqlValue::Integer(0))),
                })
                .unwrap();
            let row = store
                .scoped_get_sync(ScopedGetSpec {
                    table: "channel_member",
                    scope_col: "channel_id",
                    scope_val: text("channel-a"),
                    key_col: "user_id",
                    key_val: text(viewer),
                })
                .unwrap()
                .unwrap();
            let expected = if revision == 0 { 9 } else { 4 };
            assert!(
                matches!(row_value(&row, "unread_count"), Some(SqlValue::Integer(value)) if *value == expected)
            );
        }
    }

    #[test]
    fn equal_guard_repairs_only_same_revision_empty_identity() {
        for (revision, effect, expected) in [
            (61, "", 30),
            (60, "", 0),
            (62, "", 0),
            (61, "other", 0),
            (i64::MAX, "", 0),
        ] {
            let store = new_store(limits());
            store
                .batch_upsert_sync(UpsertSpec::new(
                    "channel_member",
                    vec![vec![
                        ("channel_id".into(), text("channel-a")),
                        ("user_id".into(), text("viewer")),
                        ("projection_revision".into(), SqlValue::Integer(revision)),
                        ("effect_id".into(), text(effect)),
                        ("updated_at".into(), SqlValue::Integer(0)),
                        ("msg_count".into(), SqlValue::Integer(0)),
                    ]],
                    Some("channel_id,user_id"),
                ))
                .unwrap();
            store
                .scoped_guarded_bump_sync(ScopedGuardedBumpSpec {
                    comparison: helix_core::effect::GuardComparison::Equal,
                    table: "channel_member",
                    scope_col: "channel_id",
                    scope_val: text("channel-a"),
                    key_col: "user_id",
                    key_val: text("viewer"),
                    bump_col: "updated_at",
                    bump_delta: 0,
                    set_cols: vec![
                        ("msg_count".into(), SqlValue::Integer(30)),
                        ("effect_id".into(), text("filled")),
                    ],
                    guard_col: "projection_revision",
                    guard_val: 61,
                    additional_guard: Some(("effect_id", text(""))),
                })
                .unwrap();
            let row = store
                .scoped_get_sync(ScopedGetSpec {
                    table: "channel_member",
                    scope_col: "channel_id",
                    scope_val: text("channel-a"),
                    key_col: "user_id",
                    key_val: text("viewer"),
                })
                .unwrap()
                .unwrap();
            assert!(
                matches!(row_value(&row,"msg_count"),Some(SqlValue::Integer(value)) if *value == expected)
            );
        }
    }

    fn limits() -> SessionStoreLimits {
        SessionStoreLimits {
            max_rows_per_table: 8,
            max_rows_per_scope: 3,
            max_scan_rows: 8,
        }
    }

    fn new_store(limits: SessionStoreLimits) -> WebSessionStore {
        WebSessionStore::new(helix_im::IM_SPARSE_TABLE_SPECS, limits, || 7_000)
            .expect("session store")
    }

    fn text(value: &str) -> SqlValue {
        SqlValue::Text(value.to_string())
    }

    fn message(temporary_id: &str, channel_id: &str, create_at: i64) -> Row {
        vec![
            ("temporary_id".into(), text(temporary_id)),
            ("channel_id".into(), text(channel_id)),
            ("create_at".into(), SqlValue::Integer(create_at)),
            ("send_status".into(), text("sent")),
        ]
    }

    /// Build a message row with an explicit value suitable for an upsert guard.
    fn message_with_props(
        temporary_id: &str,
        channel_id: &str,
        create_at: i64,
        props: &str,
    ) -> Row {
        let mut row = message(temporary_id, channel_id, create_at);
        row.push(("props".into(), text(props)));
        row
    }

    #[test]
    /// Verify sparse updates preserve excluded fields and batch atomicity.
    fn upsert_is_sparse_atomic_and_preserves_excluded_columns() {
        let store = new_store(limits());
        store
            .batch_upsert_sync(UpsertSpec {
                table: "message",
                rows: vec![message("tmp-1", "channel-a", 1)],
                conflict_key: Some("temporary_id"),
                exclude_from_update: Vec::new(),
                version_column: None,
                update_guard: None,
            })
            .expect("insert");
        store
            .batch_upsert_sync(UpsertSpec {
                table: "message",
                rows: vec![vec![
                    ("temporary_id".into(), text("tmp-1")),
                    ("channel_id".into(), text("channel-a")),
                    ("create_at".into(), SqlValue::Integer(2)),
                    ("send_status".into(), text("server-default")),
                ]],
                conflict_key: Some("temporary_id"),
                exclude_from_update: vec!["send_status"],
                version_column: None,
                update_guard: None,
            })
            .expect("update");

        let row = store
            .get_sync(GetSpec {
                table: "message",
                key_col: "temporary_id",
                key_val: text("tmp-1"),
            })
            .expect("get")
            .expect("row");
        assert!(matches!(
            row_value(&row, "create_at"),
            Some(SqlValue::Integer(2))
        ));
        assert!(matches!(row_value(&row, "send_status"), Some(SqlValue::Text(v)) if v == "sent"));

        let tight = new_store(SessionStoreLimits {
            max_rows_per_table: 8,
            max_rows_per_scope: 1,
            max_scan_rows: 8,
        });
        tight
            .batch_upsert_sync(UpsertSpec {
                table: "message",
                rows: vec![message("tmp-1", "channel-a", 1)],
                conflict_key: Some("temporary_id"),
                exclude_from_update: Vec::new(),
                version_column: None,
                update_guard: None,
            })
            .expect("first row");
        assert!(tight
            .batch_upsert_sync(UpsertSpec {
                table: "message",
                rows: vec![message("tmp-2", "channel-a", 2)],
                conflict_key: Some("temporary_id"),
                exclude_from_update: Vec::new(),
                version_column: None,
                update_guard: None,
            })
            .is_err());
        assert_eq!(tight.row_count("message").expect("count"), 1);
    }

    /// A guard only constrains an existing conflict row, so a guarded insert succeeds.
    #[test]
    fn guarded_upsert_inserts_a_missing_row_without_reading_the_guard_column() {
        let store = new_store(limits());
        store
            .batch_upsert_sync(UpsertSpec {
                table: "message",
                rows: vec![message_with_props("guarded-new", "channel-a", 1, "v1")],
                conflict_key: Some("temporary_id"),
                exclude_from_update: Vec::new(),
                version_column: None,
                update_guard: Some(UpsertGuard {
                    column: "props",
                    expected: text("stale"),
                }),
            })
            .expect("guarded insert");

        let row = store
            .get_sync(GetSpec {
                table: "message",
                key_col: "temporary_id",
                key_val: text("guarded-new"),
            })
            .expect("get")
            .expect("inserted row");
        assert!(matches!(
            row_value(&row, "props"),
            Some(SqlValue::Text(value)) if value == "v1"
        ));
    }

    /// A matching guard permits the sparse row merge and advances its payload.
    #[test]
    fn guarded_upsert_updates_when_the_expected_value_matches() {
        let store = new_store(limits());
        store
            .batch_upsert_sync(UpsertSpec {
                table: "message",
                rows: vec![message_with_props("guarded", "channel-a", 1, "v1")],
                conflict_key: Some("temporary_id"),
                exclude_from_update: Vec::new(),
                version_column: None,
                update_guard: None,
            })
            .expect("initial row");

        store
            .batch_upsert_sync(UpsertSpec {
                table: "message",
                rows: vec![message_with_props("guarded", "channel-a", 2, "v2")],
                conflict_key: Some("temporary_id"),
                exclude_from_update: Vec::new(),
                version_column: None,
                update_guard: Some(UpsertGuard {
                    column: "props",
                    expected: text("v1"),
                }),
            })
            .expect("guarded update");

        let row = store
            .get_sync(GetSpec {
                table: "message",
                key_col: "temporary_id",
                key_val: text("guarded"),
            })
            .expect("get")
            .expect("updated row");
        assert!(matches!(
            row_value(&row, "create_at"),
            Some(SqlValue::Integer(2))
        ));
        assert!(matches!(
            row_value(&row, "props"),
            Some(SqlValue::Text(value)) if value == "v2"
        ));
    }

    /// A stale expected value rejects the update without changing the existing row.
    #[test]
    fn guarded_upsert_rejects_a_stale_expected_value() {
        let store = new_store(limits());
        store
            .batch_upsert_sync(UpsertSpec {
                table: "message",
                rows: vec![message_with_props("guarded", "channel-a", 1, "v2")],
                conflict_key: Some("temporary_id"),
                exclude_from_update: Vec::new(),
                version_column: None,
                update_guard: None,
            })
            .expect("initial row");

        let error = store
            .batch_upsert_sync(UpsertSpec {
                table: "message",
                rows: vec![message_with_props("guarded", "channel-a", 3, "v3")],
                conflict_key: Some("temporary_id"),
                exclude_from_update: Vec::new(),
                version_column: None,
                update_guard: Some(UpsertGuard {
                    column: "props",
                    expected: text("v1"),
                }),
            })
            .expect_err("stale update must fail");
        assert!(matches!(error, PortError::StorageConflict));

        let row = store
            .get_sync(GetSpec {
                table: "message",
                key_col: "temporary_id",
                key_val: text("guarded"),
            })
            .expect("get")
            .expect("existing row");
        assert!(matches!(
            row_value(&row, "create_at"),
            Some(SqlValue::Integer(1))
        ));
        assert!(matches!(
            row_value(&row, "props"),
            Some(SqlValue::Text(value)) if value == "v2"
        ));
    }

    /// A guard mismatch aborts the whole atomic batch, including earlier facts.
    #[test]
    fn guarded_upsert_mismatch_rolls_back_other_atomic_facts() {
        let store = new_store(limits());
        store
            .batch_upsert_sync(UpsertSpec {
                table: "message",
                rows: vec![message_with_props("guarded", "channel-a", 1, "v2")],
                conflict_key: Some("temporary_id"),
                exclude_from_update: Vec::new(),
                version_column: None,
                update_guard: None,
            })
            .expect("initial row");

        let outcome = store.execute_atomic_write(&[
            StorageOp::BatchUpsert(UpsertSpec {
                table: "message",
                rows: vec![message_with_props("other-fact", "channel-a", 2, "new")],
                conflict_key: Some("temporary_id"),
                exclude_from_update: Vec::new(),
                version_column: None,
                update_guard: None,
            }),
            StorageOp::BatchUpsert(UpsertSpec {
                table: "message",
                rows: vec![message_with_props("guarded", "channel-a", 3, "v3")],
                conflict_key: Some("temporary_id"),
                exclude_from_update: Vec::new(),
                version_column: None,
                update_guard: Some(UpsertGuard {
                    column: "props",
                    expected: text("v1"),
                }),
            }),
        ]);
        assert!(matches!(outcome, PortOutcome::Err(_)));

        let state = store.state.lock().expect("inspect rolled-back state");
        let rows = &state.tables["message"].rows;
        assert!(
            !rows.iter().any(|stored| {
                matches!(
                    row_value(&stored.values, "temporary_id"),
                    Some(SqlValue::Text(value)) if value == "other-fact"
                )
            }),
            "earlier fact must be rolled back"
        );
        let guarded = rows
            .iter()
            .find(|stored| {
                matches!(
                    row_value(&stored.values, "temporary_id"),
                    Some(SqlValue::Text(value)) if value == "guarded"
                )
            })
            .expect("original row");
        assert!(matches!(
            row_value(&guarded.values, "props"),
            Some(SqlValue::Text(value)) if value == "v2"
        ));
    }

    #[test]
    /// Verify a capacity error leaves the whole incoming batch unapplied.
    fn capacity_failure_rolls_back_the_entire_incoming_batch() {
        let store = new_store(SessionStoreLimits {
            max_rows_per_table: 8,
            max_rows_per_scope: 1,
            max_scan_rows: 8,
        });
        assert!(store
            .batch_upsert_sync(UpsertSpec {
                table: "message",
                rows: vec![
                    message("tmp-1", "channel-a", 1),
                    message("tmp-2", "channel-a", 2),
                ],
                conflict_key: Some("temporary_id"),
                exclude_from_update: Vec::new(),
                version_column: None,
                update_guard: None,
            })
            .is_err());
        assert_eq!(store.row_count("message").expect("count"), 0);
    }

    #[test]
    /// Verify oversized batches fail before mutation.
    fn oversized_write_batch_is_rejected_before_store_state_is_copied_or_mutated() {
        let store = new_store(SessionStoreLimits {
            max_rows_per_table: 2,
            max_rows_per_scope: 2,
            max_scan_rows: 2,
        });
        let result = store.batch_upsert_sync(UpsertSpec {
            table: "message",
            rows: vec![
                message("tmp-1", "a", 1),
                message("tmp-2", "b", 2),
                message("tmp-3", "c", 3),
            ],
            conflict_key: Some("temporary_id"),
            exclude_from_update: Vec::new(),
            version_column: None,
            update_guard: None,
        });

        assert!(result.is_err());
        let state = store.state.lock().expect("state");
        assert!(state.tables["message"].rows.is_empty());
        assert_eq!(state.next_insert_seq, 0);
    }

    #[test]
    /// Verify duplicate identities cannot overwrite an excluded scope.
    fn duplicate_new_identity_keeps_first_scope_when_scope_update_is_excluded() {
        let store = new_store(SessionStoreLimits {
            max_rows_per_table: 3,
            max_rows_per_scope: 1,
            max_scan_rows: 3,
        });
        store
            .batch_upsert_sync(UpsertSpec {
                table: "message",
                rows: vec![message("existing", "channel-b", 1)],
                conflict_key: Some("temporary_id"),
                exclude_from_update: Vec::new(),
                version_column: None,
                update_guard: None,
            })
            .expect("existing row");

        store
            .batch_upsert_sync(UpsertSpec {
                table: "message",
                rows: vec![
                    message("duplicate", "channel-a", 2),
                    message("duplicate", "channel-b", 3),
                ],
                conflict_key: Some("temporary_id"),
                exclude_from_update: vec!["channel_id"],
                version_column: None,
                update_guard: None,
            })
            .expect("first scope is the final scope");

        let row = store
            .get_sync(GetSpec {
                table: "message",
                key_col: "temporary_id",
                key_val: text("duplicate"),
            })
            .expect("get")
            .expect("duplicate row");
        assert!(matches!(
            row_value(&row, "channel_id"),
            Some(SqlValue::Text(value)) if value == "channel-a"
        ));
    }

    #[test]
    fn preflight_rejects_first_scope_overflow_for_duplicate_new_identity() {
        let table = TableState {
            identity_cols: vec!["temporary_id"],
            scope_col: Some("channel_id"),
            rows: vec![StoredRow {
                values: message("existing", "channel-b", 1),
                inserted_seq: 0,
            }],
        };
        let rows = vec![
            message("duplicate", "channel-b", 2),
            message("duplicate", "channel-a", 3),
        ];

        let result = preflight_upsert_capacity(
            "message",
            &table,
            &rows,
            false,
            &["channel_id"],
            SessionStoreLimits {
                max_rows_per_table: 3,
                max_rows_per_scope: 1,
                max_scan_rows: 3,
            },
        );

        assert!(result.is_err());
    }

    #[test]
    /// Verify the legacy replacement behavior without a conflict target.
    fn no_conflict_target_replaces_the_whole_sparse_row() {
        let store = new_store(limits());
        store
            .batch_upsert_sync(UpsertSpec {
                table: "channel",
                rows: vec![vec![
                    ("id".into(), text("channel-a")),
                    ("display_name".into(), text("old")),
                    ("purpose".into(), text("removed")),
                ]],
                conflict_key: None,
                exclude_from_update: vec!["purpose"],
                version_column: None,
                update_guard: None,
            })
            .expect("insert");
        store
            .batch_upsert_sync(UpsertSpec {
                table: "channel",
                rows: vec![vec![
                    ("id".into(), text("channel-a")),
                    ("display_name".into(), text("new")),
                ]],
                conflict_key: None,
                exclude_from_update: vec!["display_name"],
                version_column: None,
                update_guard: None,
            })
            .expect("replace");

        let row = store
            .get_sync(GetSpec {
                table: "channel",
                key_col: "id",
                key_val: text("channel-a"),
            })
            .expect("get")
            .expect("channel");
        assert!(
            matches!(row_value(&row, "display_name"), Some(SqlValue::Text(value)) if value == "new")
        );
        assert!(row_value(&row, "purpose").is_none());
    }

    #[test]
    /// Verify undeclared identities fail without changing stored rows.
    fn undeclared_conflict_target_is_rejected_without_mutation() {
        let store = new_store(limits());
        assert!(store
            .batch_upsert_sync(UpsertSpec {
                table: "message",
                rows: vec![message("tmp-1", "channel-a", 1)],
                conflict_key: Some("id"),
                exclude_from_update: Vec::new(),
                version_column: None,
                update_guard: None,
            })
            .is_err());
        assert_eq!(store.row_count("message").expect("count"), 0);
    }

    #[test]
    /// Verify storage failure prevents subsequent cursor advancement.
    fn write_failure_latches_store_before_any_cursor_can_advance() {
        let store = new_store(SessionStoreLimits {
            max_rows_per_table: 8,
            max_rows_per_scope: 1,
            max_scan_rows: 8,
        });
        let failed = store.execute_ops_atomic(&[
            StorageOp::MonotonicUpsert(MonotonicUpsertSpec {
                table: "channel_event_cursor",
                key_col: "channel_id",
                value_col: "last_event_seq",
                touch_col: Some("updated_at"),
                scope_key: "channel-a".into(),
                value: 50,
            }),
            StorageOp::BatchUpsert(UpsertSpec {
                table: "message",
                rows: vec![
                    message("tmp-1", "channel-a", 1),
                    message("tmp-2", "channel-a", 2),
                ],
                conflict_key: Some("temporary_id"),
                exclude_from_update: Vec::new(),
                version_column: None,
                update_guard: None,
            }),
        ]);
        assert!(matches!(failed, PortOutcome::Err(_)));
        assert!(!store.is_healthy());

        let cursor = store.execute_ops(&[StorageOp::MonotonicUpsert(MonotonicUpsertSpec {
            table: "channel_event_cursor",
            key_col: "channel_id",
            value_col: "last_event_seq",
            touch_col: Some("updated_at"),
            scope_key: "channel-a".into(),
            value: 99,
        })]);
        assert!(matches!(cursor, PortOutcome::Err(_)));
        let scan = store.execute_ops(&[StorageOp::Scan(ScanSpec {
            table: "message",
            limit: None,
            filter: None,
            order_by: &[],
        })]);
        assert!(matches!(scan, PortOutcome::Err(_)));

        let state = store.state.lock().expect("state");
        assert!(state.tables["channel_event_cursor"].rows.is_empty());
        assert!(state.tables["message"].rows.is_empty());
    }

    #[test]
    /// Verify composite keys isolate channel writes and deletions.
    fn composite_identity_and_scoped_delete_do_not_cross_channels() {
        let store = new_store(limits());
        store
            .batch_upsert_sync(UpsertSpec {
                table: "channel_member",
                rows: vec![
                    vec![
                        ("channel_id".into(), text("a")),
                        ("user_id".into(), text("u")),
                        ("role".into(), text("member")),
                    ],
                    vec![
                        ("channel_id".into(), text("b")),
                        ("user_id".into(), text("u")),
                        ("role".into(), text("member")),
                    ],
                ],
                conflict_key: Some("channel_id,user_id"),
                exclude_from_update: Vec::new(),
                version_column: None,
                update_guard: None,
            })
            .expect("members");
        store
            .batch_delete_sync(BatchDeleteSpec {
                table: "channel_member",
                scope_col: "channel_id",
                scope_val: text("a"),
                key_col: "user_id",
                key_vals: vec![text("u")],
            })
            .expect("delete member");
        let remaining = store
            .scan_sync(ScanSpec {
                table: "channel_member",
                limit: None,
                filter: None,
                order_by: &[],
            })
            .expect("scan");
        assert_eq!(remaining.len(), 1);
        assert!(
            matches!(row_value(&remaining[0], "channel_id"), Some(SqlValue::Text(v)) if v == "b")
        );
    }

    #[test]
    /// Verify filtering, stable ordering and limiting match the storage contract.
    fn scan_filters_then_orders_then_limits_without_reordering_ties() {
        let store = new_store(limits());
        store
            .batch_upsert_sync(UpsertSpec {
                table: "message",
                rows: vec![
                    message("first", "a", 10),
                    message("second", "a", 20),
                    message("third", "a", 20),
                    message("other", "b", 99),
                ],
                conflict_key: Some("temporary_id"),
                exclude_from_update: Vec::new(),
                version_column: None,
                update_guard: None,
            })
            .expect("messages");
        let rows = store
            .scan_sync(ScanSpec {
                table: "message",
                limit: Some(2),
                filter: Some(("channel_id", text("a"))),
                order_by: CREATE_AT_DESC,
            })
            .expect("scan");
        assert_eq!(rows.len(), 2);
        assert!(
            matches!(row_value(&rows[0], "temporary_id"), Some(SqlValue::Text(v)) if v == "second")
        );
        assert!(
            matches!(row_value(&rows[1], "temporary_id"), Some(SqlValue::Text(v)) if v == "third")
        );
    }

    #[test]
    fn scoped_max_filters_scope_before_aggregating_and_ignores_empty_scope() {
        let store = new_store(limits());
        store
            .batch_upsert_sync(UpsertSpec {
                table: "message",
                rows: vec![
                    message("in-scope-1", "channel-a", 1_700),
                    message("in-scope-2", "channel-a", 2_300),
                    message("other-company", "channel-b", 9_900),
                ],
                conflict_key: Some("temporary_id"),
                exclude_from_update: Vec::new(),
                version_column: None,
                update_guard: None,
            })
            .expect("messages");

        let row = store
            .scoped_max_sync(ScopedMaxSpec {
                table: "message",
                scope_col: "channel_id",
                scope_values: vec![text("channel-a")],
                value_col: "create_at",
                result_alias: "create_at",
            })
            .expect("scoped max")
            .expect("in-scope max");
        assert!(matches!(
            row_value(&row, "create_at"),
            Some(SqlValue::Integer(2_300))
        ));
        assert!(store
            .scoped_max_sync(ScopedMaxSpec {
                table: "message",
                scope_col: "channel_id",
                scope_values: Vec::new(),
                value_col: "create_at",
                result_alias: "create_at",
            })
            .expect("empty scoped max")
            .is_none());
    }

    #[test]
    fn scoped_scan_filters_scope_before_limit_and_rejects_truncation() {
        let store = new_store(limits());
        store
            .batch_upsert_sync(UpsertSpec {
                table: "message",
                rows: vec![
                    message("in-scope-1", "channel-a", 1_700),
                    message("in-scope-2", "channel-a", 2_300),
                    message("other-company", "channel-b", 9_900),
                ],
                conflict_key: Some("temporary_id"),
                exclude_from_update: Vec::new(),
                version_column: None,
                update_guard: None,
            })
            .expect("messages");

        let rows = store
            .scoped_scan_sync(ScopedScanSpec {
                table: "message",
                scope_col: "channel_id",
                scope_values: vec![text("channel-a"), text("channel-a")],
                limit: 4,
            })
            .expect("scoped scan");
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|row| {
            matches!(row_value(row, "channel_id"), Some(SqlValue::Text(channel)) if channel == "channel-a")
        }));
        assert!(store
            .scoped_scan_sync(ScopedScanSpec {
                table: "message",
                scope_col: "channel_id",
                scope_values: vec![text("channel-a")],
                limit: 1,
            })
            .is_err());
        assert!(store
            .scoped_scan_sync(ScopedScanSpec {
                table: "missing_table",
                scope_col: "channel_id",
                scope_values: Vec::new(),
                limit: 1,
            })
            .expect("empty scope must not touch table")
            .is_empty());
    }

    #[test]
    fn monotonic_value_never_regresses_and_touch_matches_host_behavior() {
        let now = Arc::new(AtomicU64::new(100));
        let source = Arc::clone(&now);
        let store = WebSessionStore::new(helix_im::IM_SPARSE_TABLE_SPECS, limits(), move || {
            source.load(AtomicOrdering::Relaxed)
        })
        .expect("store");
        let spec = |value| MonotonicUpsertSpec {
            table: "channel_event_cursor",
            key_col: "channel_id",
            value_col: "last_event_seq",
            touch_col: Some("updated_at"),
            scope_key: "channel-a".into(),
            value,
        };
        store.monotonic_upsert_sync(spec(10)).expect("insert");
        now.store(200, AtomicOrdering::Relaxed);
        store.monotonic_upsert_sync(spec(5)).expect("smaller");
        let row = store
            .get_sync(GetSpec {
                table: "channel_event_cursor",
                key_col: "channel_id",
                key_val: text("channel-a"),
            })
            .expect("get")
            .expect("cursor");
        assert!(matches!(
            row_value(&row, "last_event_seq"),
            Some(SqlValue::Integer(10))
        ));
        assert!(matches!(
            row_value(&row, "updated_at"),
            Some(SqlValue::Integer(200))
        ));
    }

    #[test]
    fn monotonic_without_touch_does_not_read_ambient_clock() {
        let calls = Arc::new(AtomicU64::new(0));
        let source = Arc::clone(&calls);
        let store = WebSessionStore::new(helix_im::IM_SPARSE_TABLE_SPECS, limits(), move || {
            source.fetch_add(1, AtomicOrdering::Relaxed);
            123
        })
        .expect("store");
        store
            .monotonic_upsert_sync(MonotonicUpsertSpec {
                table: "channel_event_cursor",
                key_col: "channel_id",
                value_col: "last_event_seq",
                touch_col: None,
                scope_key: "channel-a".into(),
                value: 1,
            })
            .expect("monotonic");
        assert_eq!(calls.load(AtomicOrdering::Relaxed), 0);
    }

    #[test]
    /// Verify a forward bump changes its columns as one operation.
    fn guarded_bump_updates_all_columns_under_one_forward_guard() {
        let store = new_store(limits());
        store
            .batch_upsert_sync(UpsertSpec {
                table: "channel",
                rows: vec![vec![
                    ("id".into(), text("channel-a")),
                    ("unread_count".into(), SqlValue::Integer(1)),
                    ("mention_count".into(), SqlValue::Integer(0)),
                    ("last_event_seq".into(), SqlValue::Integer(10)),
                ]],
                conflict_key: None,
                exclude_from_update: Vec::new(),
                version_column: None,
                update_guard: None,
            })
            .expect("channel");
        store
            .guarded_bump_sync(GuardedBumpSpec {
                table: "channel",
                key_col: "id",
                key_val: text("channel-a"),
                bump_col: "unread_count",
                bump_delta: 1,
                extra_bumps: vec![("mention_count", 1)],
                set_cols: vec![("last_event_seq".into(), SqlValue::Integer(11))],
                guard_col: "last_event_seq",
                guard_val: 11,
            })
            .expect("bump");
        let row = store
            .get_sync(GetSpec {
                table: "channel",
                key_col: "id",
                key_val: text("channel-a"),
            })
            .expect("get")
            .expect("channel");
        assert!(matches!(
            row_value(&row, "unread_count"),
            Some(SqlValue::Integer(2))
        ));
        assert!(matches!(
            row_value(&row, "mention_count"),
            Some(SqlValue::Integer(1))
        ));
    }

    #[test]
    /// Verify rejected bumps leave no partially updated row.
    fn guarded_bump_error_does_not_leave_a_partial_row_update() {
        let store = new_store(limits());
        store
            .batch_upsert_sync(UpsertSpec {
                table: "channel",
                rows: vec![vec![
                    ("id".into(), text("channel-a")),
                    ("unread_count".into(), SqlValue::Integer(1)),
                    ("mention_count".into(), text("invalid-integer")),
                    ("last_event_seq".into(), SqlValue::Integer(10)),
                ]],
                conflict_key: Some("id"),
                exclude_from_update: Vec::new(),
                version_column: None,
                update_guard: None,
            })
            .expect("channel");
        assert!(store
            .guarded_bump_sync(GuardedBumpSpec {
                table: "channel",
                key_col: "id",
                key_val: text("channel-a"),
                bump_col: "unread_count",
                bump_delta: 1,
                extra_bumps: vec![("mention_count", 1)],
                set_cols: Vec::new(),
                guard_col: "last_event_seq",
                guard_val: 11,
            })
            .is_err());
        let row = store
            .get_sync(GetSpec {
                table: "channel",
                key_col: "id",
                key_val: text("channel-a"),
            })
            .expect("get")
            .expect("channel");
        assert!(matches!(
            row_value(&row, "unread_count"),
            Some(SqlValue::Integer(1))
        ));
    }

    #[test]
    /// Verify operation batches return the final read using the shared codec.
    fn execute_ops_returns_last_read_in_shared_core_codec() {
        let store = new_store(limits());
        let outcome = store.execute_ops(&[
            StorageOp::BatchUpsert(UpsertSpec {
                table: "message",
                rows: vec![message("tmp-1", "channel-a", 1)],
                conflict_key: Some("temporary_id"),
                exclude_from_update: Vec::new(),
                version_column: None,
                update_guard: None,
            }),
            StorageOp::Scan(ScanSpec {
                table: "message",
                limit: None,
                filter: Some(("channel_id", text("channel-a"))),
                order_by: CREATE_AT_DESC,
            }),
        ]);
        let PortOutcome::Ok(ReplyBytes(bytes)) = outcome else {
            panic!("expected successful row reply")
        };
        let value: serde_json::Value = serde_json::from_slice(&bytes).expect("rows JSON");
        assert_eq!(value.as_array().map(Vec::len), Some(1));
        assert_eq!(value[0]["temporary_id"], "tmp-1");
    }

    #[test]
    fn terminal_atomic_write_rolls_back_marker_and_rejects_read_ops() {
        let store = new_store(limits());
        let terminal = helix_im::acl::to_effect::terminal_tombstone_and_cursor_op(
            helix_im::state::ChannelId::from_str("ch00000000000000000000001a")
                .expect("valid terminal channel id"),
            helix_im::state::Seq(7),
        );
        let failed = store.execute_atomic_write(&[
            terminal.clone(),
            StorageOp::BatchUpdate(BatchUpdateSpec {
                table: "table_that_does_not_exist",
                key_col: "id",
                key_vals: vec![text("never")],
                patch: vec![("state".into(), text("closed"))],
            }),
        ]);
        assert!(matches!(failed, PortOutcome::Err(_)));
        let state = store
            .state
            .lock()
            .expect("inspect rolled-back session state");
        assert_eq!(
            state.tables["channel_event_cursor"].rows.len(),
            0,
            "failed atomic batch must roll back the tombstone marker"
        );

        let read_store = new_store(limits());
        let read_rejected = read_store.execute_atomic_write(&[StorageOp::Scan(ScanSpec {
            table: "channel_event_cursor",
            limit: None,
            filter: None,
            order_by: &[],
        })]);
        assert!(matches!(read_rejected, PortOutcome::Err(_)));
    }
    #[test]
    /// A missing row has the same JSON-array codec as HostStorage, including fresh category heads.
    fn category_review_missing_get_returns_json_array() {
        let store = new_store(limits());
        let result = store.execute_ops(&[StorageOp::Get(GetSpec {
            table: "category_chain_head",
            key_col: "scope",
            key_val: text("new-chain"),
        })]);
        let PortOutcome::Ok(bytes) = result else {
            panic!("empty read failed")
        };
        assert_eq!(bytes.0.as_ref(), b"[]");
    }

    #[test]
    /// A CAS conflict rolls back its transaction while leaving unrelated storage usable.
    fn category_review_guard_conflict_does_not_disable_store() {
        let store = new_store(limits());
        let mut spec = UpsertSpec::new(
            "message",
            vec![message_with_props("guarded", "channel-a", 1, "v2")],
            Some("temporary_id"),
        );
        assert!(matches!(
            store.execute_atomic_write(&[StorageOp::BatchUpsert(spec.clone())]),
            PortOutcome::Ok(_)
        ));
        spec.update_guard = Some(UpsertGuard {
            column: "props",
            expected: text("v1"),
        });
        assert!(matches!(
            store.execute_atomic_write(&[StorageOp::BatchUpsert(spec)]),
            PortOutcome::Err(_)
        ));
        assert!(
            store.is_healthy(),
            "a contention miss is not storage corruption"
        );
        assert!(matches!(
            store.execute_ops(&[StorageOp::Get(GetSpec {
                table: "message",
                key_col: "temporary_id",
                key_val: text("guarded"),
            })]),
            PortOutcome::Ok(_)
        ));
    }

    /// Web uses the same full-width version order and keeps the default replacement policy.
    #[test]
    fn category_review_versioned_upsert_matches_host_order() {
        let store = new_store(limits());
        for (version, expected) in [
            (10u64, 10),
            (9, 10),
            (10, 10),
            (9007199254740993, 9007199254740993),
            (u64::MAX, u64::MAX),
            (u64::MAX - 1, u64::MAX),
        ] {
            let mut spec = UpsertSpec::new(
                "message",
                vec![message_with_props(
                    "guarded",
                    "channel-a",
                    1,
                    &format!("{version:020}"),
                )],
                Some("temporary_id"),
            );
            spec.version_column = Some("props");
            assert!(matches!(
                store.execute_atomic_write(&[StorageOp::BatchUpsert(spec)]),
                PortOutcome::Ok(_)
            ));
            let outcome = store.execute_ops(&[StorageOp::Get(GetSpec {
                table: "message",
                key_col: "temporary_id",
                key_val: text("guarded"),
            })]);
            let PortOutcome::Ok(bytes) = outcome else {
                panic!("read failed")
            };
            let rows: serde_json::Value = serde_json::from_slice(&bytes.0).unwrap();
            assert_eq!(rows[0]["props"], format!("{expected:020}"));
        }
    }
}
