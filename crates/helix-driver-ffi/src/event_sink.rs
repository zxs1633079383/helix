//! FfiEventSink — 每 step 攒批 → 单次 C++ 回调过边界（HX-C007 核心）。
//!
//! ## 数据流
//!
//! pump 每 step：core 吐多条 `Effect::Emit` → dispatch 逐条调 `EventSink::emit`（此处仅入缓冲）
//! → step 末 pump 调 `BatchSink::flush`（此处编码整批 + 单次 `on_domain_events_batch`）。
//! **绝不逐条过 FFI 边界**——攒批一次过（vs Dioxus sledgehammer 逐 mutation 教训）。
//!
//! ## 缓冲并发
//!
//! `emit(&self)` 是同步 `&self`。pump 在专用单线程跑（current_thread runtime），emit 与 flush
//! 都在该线程串行调用——本无竞争。但 `EventSink: Send + Sync`（native target MaybeSend=Send）
//! 要求结构跨线程安全，故用 `Mutex<Vec<Bytes>>`（无竞争下锁开销可忽略）。

use bytes::Bytes;
use parking_lot::Mutex;

use helix_core::effect::DomainEventBytes;
use helix_core::ports::EventSink;
use helix_driver_host::BatchSink;

use crate::batch::encode_batch;
use crate::vtable::CppHostVtable;

/// FFI 领域事件 sink：攒本 step 的 Emit，flush 时整批一次回调 C++。
pub struct FfiEventSink {
    /// C++ 回调表（含 `on_domain_events_batch` + ctx）。Copy，整体持有。
    vtable: CppHostVtable,
    /// 本 step 攒下的 DomainEvent 原文缓冲；flush 后清空。
    buffer: Mutex<Vec<Bytes>>,
}

impl FfiEventSink {
    pub fn new(vtable: CppHostVtable) -> Self {
        Self {
            vtable,
            buffer: Mutex::new(Vec::new()),
        }
    }
}

impl EventSink for FfiEventSink {
    /// 仅入缓冲，不过边界（过边界推迟到 step 末的 flush，HX-C007）。
    fn emit(&self, event: DomainEventBytes) {
        self.buffer.lock().push(event.0);
    }
}

impl BatchSink for FfiEventSink {
    /// step 末：把缓冲的全部 DomainEvent 编码成单 buffer，一次调 `on_domain_events_batch`，清空。
    ///
    /// 空缓冲直接返回（pump 已对空 step 跳过 flush，此处再防御一次，避免空批回调）。
    fn flush(&self) {
        // 先取走缓冲（持锁最短），再编码 + 回调（不持锁，回调可能耗时）。
        let events = {
            let mut guard = self.buffer.lock();
            if guard.is_empty() {
                return;
            }
            std::mem::take(&mut *guard)
        };

        let Some(cb) = self.vtable.on_domain_events_batch else {
            // host 未提供回调 → 事件无出口，丢弃本批 + warn（健壮性兜底，生产应填）。
            tracing::warn!(
                count = events.len(),
                "CppHostVtable.on_domain_events_batch 为 None，丢弃本批领域事件"
            );
            return;
        };

        let buf = encode_batch(&events);
        // SAFETY: cb 由 host 经 vtable 提供，契约保证 helix_destroy 返回前有效（vtable.rs ctx 生命周期）。
        // buf 在本调用栈帧存活到回调返回后才 drop → ptr/len 在回调期间始终有效；
        // C++ 侧契约（vtable.rs 头注）保证不把 ptr 逃逸到回调返回之后。ctx 原样回传不解引用。
        unsafe {
            cb(self.vtable.ctx, buf.as_ptr(), buf.len());
        }
        // buf 在此 drop（回调已返回）。
    }
}
