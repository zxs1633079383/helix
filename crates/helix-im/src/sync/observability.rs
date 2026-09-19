//! Sync 落库诊断元数据。
//!
//! 这里只产生字段 presence/长度/hash 和 StorageOp 形状，不输出正文、用户快照或其它敏感值。
//! 元数据从可序列化的 typed view/实际 StorageOp 动态遍历，新增字段无需再维护一份日志白名单。

use helix_core::effect::{SqlValue, StorageOp};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use super::session::EventEnvelope;
use super::session::PostFields;

/// 仅在显式打开离线 sync 诊断时输出结构化 info 日志。
#[macro_export]
macro_rules! offline_sync_info {
    ($enabled:expr, $($args:tt)*) => {
        if $enabled {
            tracing::info!(target: "offline_sync", $($args)*);
        }
    };
}

/// 仅在显式打开离线 sync 诊断时输出结构化 warn 日志。
#[macro_export]
macro_rules! offline_sync_warn {
    ($enabled:expr, $($args:tt)*) => {
        if $enabled {
            tracing::warn!(target: "offline_sync", $($args)*);
        }
    };
}

/// 仅在显式打开离线 sync 诊断时输出结构化 error 日志。
#[macro_export]
macro_rules! offline_sync_error {
    ($enabled:expr, $($args:tt)*) => {
        if $enabled {
            tracing::error!(target: "offline_sync", $($args)*);
        }
    };
}

/// IM 侧只依赖的 lifecycle 阶段标签；driver 负责把它映射到真实 trace。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SyncLifecycleStage {
    T1,
    T2,
    T3,
    T4,
    T5,
}

impl SyncLifecycleStage {
    /// 返回跨端对账使用的稳定阶段标签。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::T1 => "T1",
            Self::T2 => "T2",
            Self::T3 => "T3",
            Self::T4 => "T4",
            Self::T5 => "T5",
        }
    }

    /// 将阶段映射到固定数组槽位，保持快照读取为 O(1)。
    const fn index(self) -> usize {
        match self {
            Self::T1 => 0,
            Self::T2 => 1,
            Self::T3 => 2,
            Self::T4 => 3,
            Self::T5 => 4,
        }
    }
}

/// IM 侧 lifecycle 的状态值；网络能力缺失必须表达为 not_applicable。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SyncLifecycleStatus {
    Pending,
    Started,
    Ok,
    Error,
    Skipped,
    NotApplicable,
}

impl SyncLifecycleStatus {
    /// 返回小写状态标签，避免诊断边界依赖 Debug 输出。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Started => "started",
            Self::Ok => "ok",
            Self::Error => "error",
            Self::Skipped => "skipped",
            Self::NotApplicable => "not_applicable",
        }
    }
}

/// IM 业务层可携带的最小 lifecycle 快照；不含 HTTP body、WS frame 或用户正文。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyncLifecycleContext {
    tick_id: u64,
    parent_tick_id: Option<u64>,
    stages: [SyncLifecycleStatus; 5],
    capabilities: [bool; 4],
    capability_status: [SyncLifecycleStatus; 4],
}

impl SyncLifecycleContext {
    /// 创建 T1 根快照；能力默认关闭，防止业务层伪造网络证据。
    pub fn new(tick_id: u64, parent_tick_id: Option<u64>) -> Self {
        Self {
            tick_id,
            parent_tick_id,
            stages: [
                SyncLifecycleStatus::Started,
                SyncLifecycleStatus::Skipped,
                SyncLifecycleStatus::Skipped,
                SyncLifecycleStatus::Skipped,
                SyncLifecycleStatus::Skipped,
            ],
            capabilities: [false; 4],
            capability_status: [SyncLifecycleStatus::NotApplicable; 4],
        }
    }

    /// 返回一次 sync 快照的独立 tick id。
    pub const fn tick_id(&self) -> u64 {
        self.tick_id
    }

    /// 返回可证明的上游 tick id。
    pub const fn parent_tick_id(&self) -> Option<u64> {
        self.parent_tick_id
    }

    /// 读取阶段状态，不修改当前快照。
    pub const fn stage_status(&self, stage: SyncLifecycleStage) -> SyncLifecycleStatus {
        self.stages[stage.index()]
    }

    /// 返回新快照并更新一个阶段状态。
    pub fn with_stage_status(
        &self,
        stage: SyncLifecycleStage,
        status: SyncLifecycleStatus,
    ) -> Self {
        let mut next = self.clone();
        next.stages[stage.index()] = status;
        next
    }

    /// 返回网络/持久化能力是否存在；具体能力枚举保持协议字符串稳定。
    pub fn capability_status(&self, capability: &str) -> SyncLifecycleStatus {
        let index = match capability {
            "http" => 0,
            "ws" => 1,
            "persist" => 2,
            "effect" => 3,
            _ => return SyncLifecycleStatus::NotApplicable,
        };
        self.capability_status[index]
    }

    /// 设置能力存在性；关闭能力时状态自动收敛到 not_applicable。
    pub fn with_capability(&self, capability: &str, enabled: bool) -> Self {
        let Some(index) = ["http", "ws", "persist", "effect"]
            .iter()
            .position(|value| *value == capability)
        else {
            return self.clone();
        };
        let mut next = self.clone();
        next.capabilities[index] = enabled;
        next.capability_status[index] = if enabled {
            SyncLifecycleStatus::Skipped
        } else {
            SyncLifecycleStatus::NotApplicable
        };
        next
    }

    /// 返回新快照并更新能力状态；not_applicable 同时关闭该能力。
    pub fn with_capability_status(&self, capability: &str, status: SyncLifecycleStatus) -> Self {
        let Some(index) = ["http", "ws", "persist", "effect"]
            .iter()
            .position(|value| *value == capability)
        else {
            return self.clone();
        };
        let mut next = self.clone();
        next.capabilities[index] = status != SyncLifecycleStatus::NotApplicable;
        next.capability_status[index] = status;
        next
    }

    /// 返回能力是否存在；未知能力视为不存在。
    pub fn has_capability(&self, capability: &str) -> bool {
        let Some(index) = ["http", "ws", "persist", "effect"]
            .iter()
            .position(|value| *value == capability)
        else {
            return false;
        };
        self.capabilities[index]
    }
}

impl Default for SyncLifecycleContext {
    /// 创建无上游的 synthetic root，供纯业务合同测试使用。
    fn default() -> Self {
        Self::new(0, None)
    }
}

/// 纯离线 sync 诊断配置；默认关闭且不参与业务语义。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SyncDiagnosticsConfig {
    /// 是否输出 sync/notify 事件、StorageOp 和 readback 元数据。
    pub trace: bool,
    /// 是否输出 host 侧 WAL 元数据采样。
    pub wal: bool,
    /// 目标过滤器；为空时不输出任何目标消息明细。
    pub targets: Vec<String>,
}

impl SyncDiagnosticsConfig {
    /// 返回默认关闭的诊断配置。
    pub fn disabled() -> Self {
        Self::default()
    }

    /// 判断是否允许输出指定目标的明细。
    pub(crate) fn target_matches(&self, msg_id: Option<&str>, fields: Option<&PostFields>) -> bool {
        if !self.trace || self.targets.is_empty() {
            return false;
        }
        let mut candidates = Vec::with_capacity(3);
        if let Some(msg_id) = msg_id.filter(|value| !value.is_empty()) {
            candidates.push(msg_id);
        }
        if let Some(fields) = fields {
            if !fields.id.is_empty() {
                candidates.push(fields.id.as_str());
            }
            if !fields.temporary_id.is_empty() {
                candidates.push(fields.temporary_id.as_str());
            }
        }
        self.targets.iter().any(|target| {
            let target = target.trim();
            if target.is_empty() {
                return false;
            }
            let aliases: &[&str] = match target {
                "qem" => &["qem", "helix_tmp_0000019fd1170a44_000000000000004f"],
                "bzj" => &["bzj", "helix_tmp_0000019fcb6957de_0000000000000012"],
                "sjx" => &["sjx", "helix_tmp_0000019fd1c566c6_0000000000000007"],
                _ => &[target],
            };
            candidates.iter().any(|candidate| {
                aliases.iter().any(|alias| {
                    *candidate == *alias || candidate.contains(alias) || alias.contains(candidate)
                })
            })
        })
    }
}

/// 用于把一次 sync 批次的 corr/track/source 贯穿到事件与 StorageOp 诊断日志。
#[derive(Clone, Debug)]
pub(crate) struct SyncObservation {
    pub corr: u64,
    pub track_id: String,
    pub source: &'static str,
    pub diagnostics: SyncDiagnosticsConfig,
}

impl SyncObservation {
    /// 创建一次 sync 批次的日志上下文。
    pub(crate) fn new(
        corr: u64,
        track_id: String,
        source: &'static str,
        diagnostics: SyncDiagnosticsConfig,
    ) -> Self {
        Self {
            corr,
            track_id,
            source,
            diagnostics,
        }
    }

    /// 判断该事件是否属于当前诊断目标集合。
    pub(crate) fn target_matches(&self, msg_id: Option<&str>, fields: Option<&PostFields>) -> bool {
        self.diagnostics.target_matches(msg_id, fields)
    }
}

/// 为 PostFields 生成脱敏的动态字段摘要。
pub(crate) fn post_fields_metadata(fields: &PostFields) -> Value {
    let value = serde_json::to_value(fields).unwrap_or_else(|_| Value::Object(Map::new()));
    value_metadata(&value)
}

/// 统计批次中真实出现的事件类型，供事务前后摘要复用。
pub(crate) fn event_type_counts(events: &[EventEnvelope]) -> Value {
    let mut counts = Map::new();
    for event_type in [1_u8, 2, 3, 6, 7] {
        let count = events
            .iter()
            .filter(|event| event.kind.type_num() == event_type)
            .count();
        counts.insert(format!("type{event_type}"), Value::from(count));
    }
    Value::Object(counts)
}

/// 统计 PersistAtomic 中按表/patch 形状识别出的类型计数。
pub(crate) fn storage_op_type_counts(ops: &[StorageOp]) -> Value {
    let mut counts = Map::new();
    for key in ["type1", "type2", "type3", "type6", "other"] {
        counts.insert(key.to_string(), Value::from(0_u64));
    }
    for op in ops {
        let key = match op {
            StorageOp::BatchUpsert(spec) if spec.table == "message" => "type1",
            StorageOp::BatchUpdate(spec)
                if spec.table == "message"
                    && spec.patch.iter().any(|(column, _)| column == "revoke") =>
            {
                "type3"
            }
            StorageOp::BatchUpdate(spec)
                if spec.table == "message"
                    && spec.patch.iter().any(|(column, _)| column == "read_bits") =>
            {
                "type6"
            }
            StorageOp::BatchUpdate(spec) if spec.table == "message" => "type2",
            _ => "other",
        };
        let count = counts.get(key).and_then(Value::as_u64).unwrap_or(0);
        counts.insert(key.to_string(), Value::from(count + 1));
    }
    Value::Object(counts)
}

/// 为实际 StorageOp 生成脱敏的操作摘要，patch_columns 始终来自操作本身。
pub(crate) fn storage_op_metadata(op: &StorageOp) -> Value {
    match op {
        StorageOp::BatchUpsert(spec) => {
            let row = spec.rows.first();
            let mut result = json!({
                "operation": "BatchUpsert",
                "table": spec.table,
                "conflict_key": spec.conflict_key,
                "patch_columns": row.map(|row| row.iter().map(|(key, _)| key).collect::<Vec<_>>()).unwrap_or_default(),
            });
            if let Some(row) = row {
                result["patch_field_meta"] =
                    row_metadata(row.iter().map(|(key, value)| (key.as_str(), value)));
            }
            result
        }
        StorageOp::BatchUpdate(spec) => json!({
            "operation": "BatchUpdate",
            "table": spec.table,
            "key_col": spec.key_col,
            "key_count": spec.key_vals.len(),
            "patch_columns": spec.patch.iter().map(|(key, _)| key).collect::<Vec<_>>(),
            "patch_field_meta": row_metadata(spec.patch.iter().map(|(key, value)| (key.as_str(), value))),
        }),
        StorageOp::MonotonicUpsert(spec) => json!({
            "operation": "MonotonicUpsert",
            "table": spec.table,
            "key_col": spec.key_col,
            "value_col": spec.value_col,
            "key_count": 1,
        }),
        StorageOp::GuardedBump(spec) => json!({
            "operation": "GuardedBump",
            "table": spec.table,
            "key_col": spec.key_col,
            "bump_col": spec.bump_col,
            "patch_columns": spec.set_cols.iter().map(|(key, _)| key).collect::<Vec<_>>(),
        }),
        StorageOp::ScopedGuardedBump(spec) => json!({
            "operation": "ScopedGuardedBump",
            "table": spec.table,
            "scope_col": spec.scope_col,
            "key_col": spec.key_col,
            "bump_col": spec.bump_col,
            "patch_columns": spec.set_cols.iter().map(|(key, _)| key).collect::<Vec<_>>(),
        }),
        StorageOp::BatchDelete(spec) => json!({
            "operation": "BatchDelete",
            "table": spec.table,
            "scope_col": spec.scope_col,
            "key_col": spec.key_col,
            "key_count": spec.key_vals.len(),
        }),
        StorageOp::Get(spec) => json!({ "operation": "Get", "table": spec.table }),
        StorageOp::ScopedGet(spec) => json!({ "operation": "ScopedGet", "table": spec.table }),
        StorageOp::ScopedMax(spec) => json!({
            "operation": "ScopedMax",
            "table": spec.table,
            "scope_col": spec.scope_col,
            "value_col": spec.value_col,
            "result_alias": spec.result_alias,
            "scope_count": spec.scope_values.len(),
        }),
        StorageOp::ScopedScan(spec) => json!({
            "operation": "ScopedScan",
            "table": spec.table,
            "scope_col": spec.scope_col,
            "scope_count": spec.scope_values.len(),
            "limit": spec.limit,
        }),
        StorageOp::Scan(spec) => json!({ "operation": "Scan", "table": spec.table }),
    }
}

/// 把事件字段/StorageOp 的真实值转换为 kind/length/hash，不泄露原始值。
fn value_metadata(value: &Value) -> Value {
    let mut presence = Map::new();
    let mut lengths = Map::new();
    let mut hashes = Map::new();
    if let Value::Object(fields) = value {
        for (key, value) in fields {
            let present = value_is_present(value);
            presence.insert(key.clone(), Value::Bool(present));
            if present {
                if let Some(length) = value_length(value) {
                    lengths.insert(key.clone(), Value::from(length));
                }
                hashes.insert(key.clone(), Value::String(sha256_value(value)));
            }
        }
    }
    json!({
        "field_presence": presence,
        "field_lengths": lengths,
        "field_hashes": hashes,
    })
}

/// 生成 StorageOp patch 的字段级脱敏元数据。
fn row_metadata<'a>(row: impl Iterator<Item = (&'a str, &'a SqlValue)>) -> Value {
    let mut metadata = Map::new();
    for (key, value) in row {
        metadata.insert(key.to_string(), sql_value_metadata(value));
    }
    Value::Object(metadata)
}

fn sql_value_metadata(value: &SqlValue) -> Value {
    match value {
        SqlValue::Null => json!({ "kind": "null" }),
        SqlValue::Integer(value) => json!({
            "kind": "integer",
            "hash": sha256_bytes(value.to_string().as_bytes()),
        }),
        SqlValue::Real(value) => json!({
            "kind": "real",
            "hash": sha256_bytes(value.to_string().as_bytes()),
        }),
        SqlValue::Text(value) => json!({
            "kind": "text",
            "length": value.len(),
            "hash": sha256_bytes(value.as_bytes()),
        }),
        SqlValue::Blob(value) => json!({
            "kind": "blob",
            "length": value.len(),
            "hash": sha256_bytes(value),
        }),
    }
}

fn value_is_present(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::String(value) => !value.is_empty(),
        Value::Array(value) => !value.is_empty(),
        Value::Object(value) => !value.is_empty(),
        Value::Number(value) => value.as_i64().is_none_or(|number| number != 0),
        Value::Bool(value) => *value,
    }
}

fn value_length(value: &Value) -> Option<usize> {
    match value {
        Value::String(value) => Some(value.len()),
        Value::Array(value) => Some(value.len()),
        Value::Object(value) => Some(value.len()),
        _ => None,
    }
}

fn sha256_value(value: &Value) -> String {
    let encoded = serde_json::to_vec(value).unwrap_or_default();
    sha256_bytes(&encoded)
}

fn sha256_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("sha256:{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_discovers_new_fields_without_logging_values() {
        let fields = PostFields {
            msg_type: "TEXT".to_string(),
            message: "secret body".to_string(),
            expedite_map: "{\"678\":true}".to_string(),
            ..PostFields::default()
        };
        let metadata = post_fields_metadata(&fields);
        assert_eq!(metadata["field_presence"]["type"], true);
        assert_eq!(metadata["field_presence"]["message"], true);
        assert_eq!(metadata["field_presence"]["expedite_map"], true);
        assert!(metadata["field_hashes"]["message"]
            .as_str()
            .is_some_and(|hash| hash.starts_with("sha256:")));
        assert!(!metadata.to_string().contains("secret body"));
    }

    #[test]
    fn diagnostics_are_disabled_without_explicit_targets() {
        let config = SyncDiagnosticsConfig::disabled();
        assert!(!config.trace);
        assert!(!config.wal);
        assert!(!config.target_matches(Some("qem"), None));
    }

    #[test]
    fn diagnostics_accept_target_alias_and_temporary_id() {
        let config = SyncDiagnosticsConfig {
            trace: true,
            wal: true,
            targets: vec!["qem".to_string()],
        };
        let fields = PostFields {
            temporary_id: "helix_tmp_0000019fd1170a44_000000000000004f".to_string(),
            ..PostFields::default()
        };
        assert!(config.target_matches(None, Some(&fields)));
        assert!(!config.target_matches(Some("other"), None));
    }

    #[test]
    fn lifecycle_context_keeps_t1_to_t5_and_tick_parent_explicit() {
        let root = SyncLifecycleContext::new(41, None).with_capability("http", true);
        let next = root
            .with_stage_status(SyncLifecycleStage::T3, SyncLifecycleStatus::Started)
            .with_stage_status(SyncLifecycleStage::T4, SyncLifecycleStatus::Skipped);
        let child = SyncLifecycleContext::new(42, Some(root.tick_id()));

        assert_eq!(next.tick_id(), 41);
        assert_eq!(next.parent_tick_id(), None);
        assert_eq!(child.parent_tick_id(), Some(41));
        assert_eq!(
            next.stage_status(SyncLifecycleStage::T3),
            SyncLifecycleStatus::Started
        );
        assert_eq!(
            next.stage_status(SyncLifecycleStage::T4),
            SyncLifecycleStatus::Skipped
        );
        assert_eq!(
            next.capability_status("ws"),
            SyncLifecycleStatus::NotApplicable
        );
        assert_eq!(next.capability_status("http"), SyncLifecycleStatus::Skipped);
    }

    #[test]
    fn lifecycle_capability_matrix_does_not_promote_absent_network_to_ok() {
        let no_network = SyncLifecycleContext::new(1, None);
        let http_only = no_network.with_capability("http", true);
        let ws_only = no_network.with_capability("ws", true);

        assert_eq!(
            no_network.capability_status("http"),
            SyncLifecycleStatus::NotApplicable
        );
        assert_eq!(
            no_network.capability_status("ws"),
            SyncLifecycleStatus::NotApplicable
        );
        assert_eq!(
            http_only.capability_status("http"),
            SyncLifecycleStatus::Skipped
        );
        assert_eq!(
            http_only.capability_status("ws"),
            SyncLifecycleStatus::NotApplicable
        );
        assert_eq!(
            ws_only.capability_status("ws"),
            SyncLifecycleStatus::Skipped
        );
        assert_eq!(
            ws_only.capability_status("http"),
            SyncLifecycleStatus::NotApplicable
        );
    }
}
