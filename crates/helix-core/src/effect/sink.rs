use super::Effect;

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
    diagnostics: crate::diagnostics::StepDiagnostics,
}

impl EffectSink {
    pub fn new() -> Self {
        Self {
            buf: Vec::with_capacity(8),
            diagnostics: Default::default(),
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
        self.diagnostics.clear();
    }

    /// 读取本次 step 产出的全部 Effect
    #[inline]
    pub fn as_slice(&self) -> &[Effect] {
        &self.buf
    }

    /// 在原缓冲中筛选/替换本次模块追加的输出，不触及先前模块的输出、不重新分配缓冲。
    pub fn retain_mut_from(&mut self, start: usize, mut keep: impl FnMut(&mut Effect) -> bool) {
        let mut index = 0;
        self.buf.retain_mut(|effect| {
            let retained = index < start || keep(effect);
            index += 1;
            retained
        });
    }

    /// 只借用本轮后缀进行原地访问；不遍历前缀、不移除元素、不分配缓冲。
    pub fn visit_mut_from(&mut self, start: usize, visitor: impl FnMut(&mut Effect)) {
        self.buf
            .get_mut(start..)
            .unwrap_or(&mut [])
            .iter_mut()
            .for_each(visitor);
    }

    /// 借用当前轮诊断；与Effect缓冲同时清理，禁止跨轮串线。
    pub fn diagnostics(&self) -> &crate::diagnostics::StepDiagnostics {
        &self.diagnostics
    }

    /// 注入诊断上下文或上报失败，不改变业务Effect。
    pub fn diagnostics_mut(&mut self) -> &mut crate::diagnostics::StepDiagnostics {
        &mut self.diagnostics
    }

    /// 分离借用Effect后缀和诊断缓冲，失败可直接报告而无需临时集合。
    pub fn visit_with_diagnostics_from(
        &mut self,
        start: usize,
        mut visitor: impl FnMut(&mut Effect, &mut crate::diagnostics::StepDiagnostics),
    ) {
        for effect in self.buf.get_mut(start..).unwrap_or(&mut []) {
            visitor(effect, &mut self.diagnostics);
        }
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
    use crate::effect::{DomainEventBytes, TimerId};

    /// 出口过滤只处理本模块追加的后缀，并保留其它 Effect 的原始顺序。
    #[test]
    fn retain_suffix_preserves_prefix_and_nonmatching_effects() {
        let mut sink = EffectSink::new();
        sink.push(Effect::Emit {
            event: DomainEventBytes(bytes::Bytes::from_static(b"prefix")),
        });
        sink.push(Effect::Emit {
            event: DomainEventBytes(bytes::Bytes::from_static(b"remove")),
        });
        sink.push(Effect::CancelTimer {
            id: TimerId::from_raw(1),
        });
        sink.retain_mut_from(1, |effect| !matches!(effect, Effect::Emit { .. }));
        assert_eq!(sink.as_slice().len(), 2);
        assert!(matches!(&sink.as_slice()[0],Effect::Emit{event} if event.0.as_ref()==b"prefix"));
        assert!(matches!(&sink.as_slice()[1], Effect::CancelTimer { .. }));
    }
}
