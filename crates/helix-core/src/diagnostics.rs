//! 宿主显式注入的诊断数据；不读取 ambient span，不依赖 OTel 或运行时。

/// 已验证的非零 W3C TraceId；Copy 值只用于当前 step 的诊断。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TraceId([u8; 32]);

impl TraceId {
    /// 接受32位小写十六进制；非法/全零标识不会进入客户端错误。
    pub fn from_hex(value: &str) -> Option<Self> {
        let bytes: [u8; 32] = value.as_bytes().try_into().ok()?;
        (bytes
            .iter()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b))
            && bytes.iter().any(|b| *b != b'0'))
        .then_some(Self(bytes))
    }

    /// 返回验证过的ASCII标识，无分配。
    pub fn as_str(&self) -> &str {
        std::str::from_utf8(&self.0).expect("TraceId contains validated ASCII")
    }
}

/// 单条及整批错误文本的UTF-8字节预算；数量另限16条。
pub const FAILURE_TEXT_BYTES: usize = 4096;

/// 模块交给宿主的不可变诊断；私有文本保证只能通过有界入口构造。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FailureDiagnostic {
    pub kind: &'static str,
    message: std::sync::Arc<str>,
}

impl FailureDiagnostic {
    /// 最多复制4KiB，UTF-8边界最多回退3字节；不扫描完整输入。
    pub fn new(kind: &'static str, message: &str) -> Self {
        Self::with_limit(kind, message, FAILURE_TEXT_BYTES)
    }

    fn with_limit(kind: &'static str, message: &str, limit: usize) -> Self {
        let mut end = message.len().min(limit);
        while !message.is_char_boundary(end) {
            end -= 1;
        }
        Self {
            kind,
            message: std::sync::Arc::from(&message[..end]),
        }
    }

    /// 借用已截断的文本，不分配。
    pub fn message(&self) -> &str {
        &self.message
    }

    /// SDK支持Arc<str>属性值；克隆此指针不会再次复制文本。
    pub fn shared_message(&self) -> &std::sync::Arc<str> {
        &self.message
    }
}

/// 同时约束条数和总字节的诊断批次；共享到span和SDK时不重做截断。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FailureBuffer {
    entries: Vec<FailureDiagnostic>,
    bytes: usize,
    dropped: u32,
}

impl FailureBuffer {
    /// 只共享合法且预算内的诊断；超额保留计数，由宿主保持Error状态。
    pub fn push(&mut self, failure: &FailureDiagnostic) {
        if self.entries.len() < 16 && failure.message().len() <= FAILURE_TEXT_BYTES - self.bytes {
            self.bytes += failure.message().len();
            self.entries.push(failure.clone());
        } else {
            self.add_dropped(1);
        }
    }

    /// 已省略的诊断数量，映射到OTLP dropped_events_count。
    pub fn dropped(&self) -> u32 {
        self.dropped
    }

    /// 合并上游省略计数，不创建伪exception或修改错误状态。
    pub fn add_dropped(&mut self, count: u32) {
        self.dropped = self.dropped.saturating_add(count);
    }

    /// 原始文本只在此截断一次；后续消费者共享所有权。
    fn report(&mut self, kind: &'static str, message: &str) {
        let remaining = FAILURE_TEXT_BYTES - self.bytes;
        if self.entries.len() == 16 || (remaining == 0 && !message.is_empty()) {
            self.add_dropped(1);
            return;
        }
        let failure = FailureDiagnostic::with_limit(kind, message, remaining);
        if failure.message().is_empty() && !message.is_empty() {
            self.add_dropped(1);
            return;
        }
        self.bytes += failure.message().len();
        self.entries.push(failure);
    }

    fn clear(&mut self) {
        self.entries.clear();
        self.bytes = 0;
        self.dropped = 0;
    }
}

impl std::ops::Deref for FailureBuffer {
    type Target = [FailureDiagnostic];
    fn deref(&self) -> &Self::Target {
        &self.entries
    }
}

/// 每轮复用的诊断；成功路径不分配，不保存跨轮span。
#[derive(Default)]
pub struct StepDiagnostics {
    pub(crate) trace_id: Option<TraceId>,
    failures: FailureBuffer,
}

impl StepDiagnostics {
    /// 当前step由宿主提供的TraceId，未启用追踪时为None。
    pub fn trace_id(&self) -> Option<TraceId> {
        self.trace_id
    }

    /// 只有开启追踪才保存失败；超限不影响业务返回。
    pub fn report(&mut self, kind: &'static str, message: &str) {
        if self.trace_id.is_some() {
            self.failures.report(kind, message);
        }
    }

    /// 宿主借用诊断，不搬运或解析业务payload。
    pub fn failures(&self) -> &[FailureDiagnostic] {
        &self.failures
    }

    /// 上游预算省略数量，供当前span记录丢弃事件数。
    pub fn dropped_failures(&self) -> u32 {
        self.failures.dropped()
    }

    pub(crate) fn clear(&mut self) {
        self.trace_id = None;
        self.failures.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    /// 边界不足一个UTF-8字符时省略该事件，空文本仍受数量限制。
    #[test]
    fn partial_utf8_and_empty_failures_obey_both_limits() {
        let mut buffer = FailureBuffer::default();
        buffer.report("first", &"x".repeat(4095));
        buffer.report("unicode", "🦀");
        assert_eq!(buffer.len(), 1);
        assert_eq!(buffer.dropped(), 1);
        buffer.clear();
        for _ in 0..20 {
            buffer.report("empty", "");
        }
        assert_eq!(buffer.len(), 16);
        assert_eq!(buffer.dropped(), 4);
        assert_eq!(buffer.bytes, 0);
    }

    /// 多字节文本的限制必须按字节而非字符计算；复制诊断必须共享原缓冲。
    #[test]
    fn utf8_budget_and_shared_clone() {
        let mut diagnostics = StepDiagnostics::default();
        diagnostics.trace_id = TraceId::from_hex("11111111111111111111111111111111");
        diagnostics.report("failed", &"🦀".repeat(4096));
        let failure = &diagnostics.failures()[0];
        assert!(failure.message.len() <= 4096);
        let cloned = failure.clone();
        assert_eq!(failure.message.as_ptr(), cloned.message.as_ptr());
    }

    /// 连续失败受整轮字节预算限制，不能仅逐条截断后仍积累64KiB。
    #[test]
    fn total_failure_bytes_are_bounded() {
        let mut diagnostics = StepDiagnostics::default();
        diagnostics.trace_id = TraceId::from_hex("11111111111111111111111111111111");
        for _ in 0..16 {
            diagnostics.report("failed", &"x".repeat(4096));
        }
        assert!(
            diagnostics
                .failures()
                .iter()
                .map(|f| f.message.len())
                .sum::<usize>()
                <= 4096
        );
    }

    #[test]
    fn invalid_ids_are_rejected_and_buffer_is_bounded_and_cleared() {
        for id in [
            "",
            "00000000000000000000000000000000",
            "GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG",
            "é111111111111111111111111111111",
        ] {
            assert!(TraceId::from_hex(id).is_none());
        }
        let mut diagnostics = StepDiagnostics::default();
        diagnostics.report("failed", "ignored without consumer");
        assert!(diagnostics.failures().is_empty());
        diagnostics.trace_id = TraceId::from_hex("11111111111111111111111111111111");
        for _ in 0..32 {
            diagnostics.report("failed", &"x".repeat(8192));
        }
        assert_eq!(diagnostics.failures().len(), 1);
        assert_eq!(diagnostics.dropped_failures(), 31);
        assert_eq!(diagnostics.failures()[0].message.len(), 4096);
        diagnostics.clear();
        assert!(diagnostics.trace_id().is_none());
        assert!(diagnostics.failures().is_empty());
    }
}
