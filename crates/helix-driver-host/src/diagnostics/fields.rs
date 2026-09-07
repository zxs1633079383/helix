//! 白名单固定栈缓冲；序号/身份超长整条拒绝，不记录正文和任意debug对象。
use std::fmt::Write;
use tracing::field::{Field, Visit};
const NAMES: &[&str] = &[
    "event",
    "business_event_id",
    "message_id",
    "batch_id",
    "traceparent",
    "tenant_id",
    "user_id",
    "channel_id",
    "login_attempt_id",
    "device_session_id",
    "stage",
    "path",
    "result",
    "reason",
    "seq_domain",
    "stream_epoch",
    "event_seq",
    "local_contiguous_seq",
    "observed_high_seq",
    "expected_seq",
    "received_seq",
    "declared_from_seq",
    "declared_to_seq",
    "corr",
    "count",
    "returned_channels",
    "persisted_channels",
    "applied_channels",
    "evidence_complete",
    "expected_known",
    "sync_session_id",
    "has_more",
    "source",
    "gap_id",
    "elapsed_seconds",
    "generation",
    "from_seq",
    "next_seq",
    "server_horizon_seq",
];
#[derive(Clone, Copy)]
struct Text {
    bytes: [u8; 128],
    len: usize,
    present: bool,
}
impl Default for Text {
    /// 固定零值，无堆分配。
    fn default() -> Self {
        Self {
            bytes: [0; 128],
            len: 0,
            present: false,
        }
    }
}
impl std::fmt::Write for Text {
    /// 超长立即失败，避免身份和序号被截断后仍导出。
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        if self.len + s.len() > self.bytes.len() {
            return Err(std::fmt::Error);
        }
        self.bytes[self.len..self.len + s.len()].copy_from_slice(s.as_bytes());
        self.len += s.len();
        self.present = true;
        Ok(())
    }
}
pub(super) struct Fields {
    values: [Text; NAMES.len()],
    pub invalid: bool,
    pub observed_ns: u128,
}
impl Default for Fields {
    /// 字段容量与白名单同步，避免新增字段越界；每个标量最多128字节。
    fn default() -> Self {
        Self {
            values: [Text::default(); NAMES.len()],
            invalid: false,
            observed_ns: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos(),
        }
    }
}
impl Fields {
    /// 仅具备明确事件类型的白名单记录可发送。
    pub fn has_event(&self) -> bool {
        self.values[0].present && self.values[0].len > 0
    }
    /// 后台线程将受限标量转换JSON；序号和ID始终字符串。
    pub fn json(&self) -> serde_json::Map<String, serde_json::Value> {
        let mut out = serde_json::Map::new();
        for (name, value) in NAMES.iter().zip(self.values.iter()) {
            if !value.present || value.len == 0 {
                continue;
            }
            let text = std::str::from_utf8(&value.bytes[..value.len]).unwrap_or("");
            let value = match *name {
                "evidence_complete" | "expected_known" | "has_more" => {
                    serde_json::Value::Bool(text == "true")
                }
                "count" | "returned_channels" | "persisted_channels" | "applied_channels" => text
                    .parse::<u64>()
                    .map(serde_json::Value::from)
                    .unwrap_or(serde_json::Value::Null),
                "elapsed_seconds" => text
                    .parse::<f64>()
                    .ok()
                    .and_then(serde_json::Number::from_f64)
                    .map(serde_json::Value::Number)
                    .unwrap_or(serde_json::Value::Null),
                _ => serde_json::Value::String(text.to_owned()),
            };
            out.insert((*name).to_owned(), value);
        }
        out
    }
    /// 未知字段直接忽略；已知字段写入固定栈缓冲。
    fn put(&mut self, field: &Field, value: std::fmt::Arguments<'_>) {
        if let Some(i) = NAMES.iter().position(|name| *name == field.name()) {
            if self.values[i].write_fmt(value).is_err() {
                self.invalid = true;
            }
        }
    }
}
impl Visit for Fields {
    /// 数值在业务线程只格式化到栈，JSON类型由后台白名单决定。
    fn record_u64(&mut self, f: &Field, v: u64) {
        self.put(f, format_args!("{v}"));
    }
    /// 负数保留用于失败诊断，不用于无符号业务水位。
    fn record_i64(&mut self, f: &Field, v: i64) {
        self.put(f, format_args!("{v}"));
    }
    /// 延迟由宿主时钟给出。
    fn record_f64(&mut self, f: &Field, v: f64) {
        self.put(f, format_args!("{v}"));
    }
    /// 布尔事实保留原值。
    fn record_bool(&mut self, f: &Field, v: bool) {
        self.put(f, format_args!("{v}"));
    }
    /// 仅接收白名单字符串字段，禁止任意消息正文。
    fn record_str(&mut self, f: &Field, v: &str) {
        self.put(f, format_args!("{v}"));
    }
    /// Display包装的ID/可选序号只进入白名单固定缓冲。
    fn record_debug(&mut self, f: &Field, v: &dyn std::fmt::Debug) {
        self.put(f, format_args!("{v:?}"));
    }
}
