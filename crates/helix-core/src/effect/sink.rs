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
}

impl EffectSink {
    pub fn new() -> Self {
        Self {
            buf: Vec::with_capacity(8),
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
    }

    /// 读取本次 step 产出的全部 Effect
    #[inline]
    pub fn as_slice(&self) -> &[Effect] {
        &self.buf
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
