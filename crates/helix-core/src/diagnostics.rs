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

/// 模块交给宿主的失败诊断；不改变业务Effect或承担业务恢复。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FailureDiagnostic {
    pub kind: &'static str,
    pub message: String,
}

/// 每轮复用、最多保留16个失败；成功路径不分配，不保存跨轮span。
#[derive(Default)]
pub struct StepDiagnostics {
    pub(crate) trace_id: Option<TraceId>,
    failures: Vec<FailureDiagnostic>,
}

impl StepDiagnostics {
    /// 当前step由宿主提供的TraceId，未启用追踪时为None。
    pub fn trace_id(&self) -> Option<TraceId> {
        self.trace_id
    }

    /// 只有开启追踪才保存失败；超限不会影响业务返回。
    pub fn report(&mut self, kind: &'static str, message: &str) {
        if self.trace_id.is_some() && self.failures.len() < 16 {
            self.failures.push(FailureDiagnostic {
                kind,
                message: message.chars().take(4096).collect(),
            });
        }
    }

    /// 宿主借用诊断，不搬运或解析业务payload。
    pub fn failures(&self) -> &[FailureDiagnostic] {
        &self.failures
    }

    pub(crate) fn clear(&mut self) {
        self.trace_id = None;
        self.failures.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
        assert_eq!(diagnostics.failures().len(), 16);
        assert_eq!(diagnostics.failures()[0].message.len(), 4096);
        diagnostics.clear();
        assert!(diagnostics.trace_id().is_none());
        assert!(diagnostics.failures().is_empty());
    }
}
