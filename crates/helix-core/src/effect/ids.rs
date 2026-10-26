use bytes::Bytes;

/// 不透明令牌：把 `Effect::Persist` / `Effect::Http` 与未来的 `Tick::PortReply` 配对。
///
/// `ExecutionShell` 维护 `Correlation → module_index` 映射（`corr_map`），
/// 保证 PortReply 定向投递给发出该 Correlation 的模块（审查意见2 P3 落实）。
///
/// ## MIN-1 修复
///
/// 内部值改为 `pub(crate)` — 外部 crate 通过 `raw()` 只读访问，
/// 只有 engine 的 `CorrelationSource` 可以创建新值，保证"ID 由分配器单调发放"不变量。
/// helix-im 通过 `ImModule::alloc_corr()` 获取（`helix_core::Correlation::new_from_raw`
/// 是 pub(crate) 构造器，只对 helix-core 自身可见）。
///
/// 由于 helix-im 是独立 crate，需要分配 Correlation，保留一个 `pub` 的 `from_raw`
/// 构造器（但语义上调用方应通过模块内的 IdAllocator 管理，不得随意构造）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Correlation(pub(crate) u64);

impl Correlation {
    /// 从原始 u64 构造（供 helix-im 等业务 crate 的 IdAllocator 使用）。
    ///
    /// 调用方负责保证全局单调唯一性；不得在 ID 分配器之外随意调用。
    pub fn from_raw(v: u64) -> Self {
        Self(v)
    }

    /// 获取内部原始值（只读）
    pub fn raw(self) -> u64 {
        self.0
    }
}

/// 不透明定时器 ID：把 `Effect::ScheduleTimer` 与未来的 `Tick::Timer` 配对。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TimerId(pub(crate) u64);

impl TimerId {
    /// 从原始 u64 构造（供业务 crate 的 IdAllocator 使用）
    pub fn from_raw(v: u64) -> Self {
        Self(v)
    }

    /// 获取内部原始值（只读）
    pub fn raw(self) -> u64 {
        self.0
    }
}

/// 传输通道 ID（模块可管理多个 transport，如 WS + debug transport）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TransportId(pub(crate) u32);

impl TransportId {
    pub fn from_raw(v: u32) -> Self {
        Self(v)
    }

    pub fn raw(self) -> u32 {
        self.0
    }
}

/// 零拷贝事件字节。`bytes::Bytes` 引用计数，clone = O(1)。
/// core 不解析内容，只搬运已序列化的字节。
#[derive(Debug, Clone)]
pub struct EncodedDomainEvent(pub Bytes, bool);

/// 零拷贝事件载体；观测标记只在 Rust 内存中传递，不进入 wire bytes。
/// 使用兼容构造函数 `DomainEventBytes(bytes)` 或 `DomainEventBytes::observed(bytes)`。
pub type DomainEventBytes = EncodedDomainEvent;

/// 保留既有字节构造调用；默认不需要模块层的额外观测。
#[allow(non_snake_case)]
pub fn DomainEventBytes(bytes: Bytes) -> DomainEventBytes {
    EncodedDomainEvent(bytes, false)
}

impl EncodedDomainEvent {
    /// producer 在编码前确定需要额外观测；core 不读取或判断业务内容。
    pub fn observed(bytes: Bytes) -> Self {
        Self(bytes, true)
    }

    /// 借用内部标记，无需重新解析 bytes；克隆和移动均保留标记。
    pub fn needs_observation(&self) -> bool {
        self.1
    }
}

#[cfg(test)]
mod observation_tests {
    use super::*;
    /// 标记随所有权移动和Bytes共享克隆保留，wire内容完全相同。
    #[test]
    fn observation_hint_is_private_metadata_not_wire() {
        let plain = DomainEventBytes(Bytes::from_static(b"opaque"));
        let observed = DomainEventBytes::observed(plain.0.clone());
        assert!(!plain.needs_observation());
        assert!(observed.clone().needs_observation());
        assert_eq!(observed.0, plain.0);
    }
}
