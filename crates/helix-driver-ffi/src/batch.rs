//! batch — 一个 step 的多条 DomainEventBytes 编码成单 buffer 的 wire layout（HX-C007）。
//!
//! ## 为什么批
//!
//! core 一个 step 可吐多条 `Effect::Emit`。逐条过 FFI 边界（每条一次 `on_domain_events_batch`）
//! 在高频场景下是数量级开销（Dioxus sledgehammer 教训）。故攒一个 step 的全部事件，编码成
//! **单个 byte buffer**，一次回调过边界。
//!
//! ## Wire layout（小端，自描述长度前缀）
//!
//! ```text
//! ┌──────────────┬─────────────────────────────────────────────┐
//! │ u32 count    │ 事件条数                                       │
//! ├──────────────┼──────────────┬──────────────────────────────┤
//! │ 每条:        │ u32 len      │ len 字节 DomainEventBytes 原文 │
//! └──────────────┴──────────────┴──────────────────────────────┘
//! ```
//!
//! - 全部整数 **little-endian**（C++ 侧按 LE 读；移动端 ARM 默认 LE，x86 亦 LE）。
//! - `count == 0` → 仅 4 字节 header（pump 只在非空批时回调，正常不出现空批）。
//! - 每条原文 = helix-im 已序列化的 `{"event":...,"data":...}` JSON 字节，FFI 不再解析。
//! - C++ 侧解码：读 count → 循环读 (len, bytes)。`ptr` 仅回调期间有效，C++ 须立即拷贝/消费。

use bytes::Bytes;

/// 把一批 `DomainEventBytes` 原文编码成单 buffer（见模块头 wire layout）。
///
/// 返回 owned `Vec<u8>`；调用方（FfiEventSink::flush）取其 `as_ptr()/len()` 传 C++ 回调，
/// buffer 生命周期由调用方栈帧持有到回调返回后再 drop（C++ 不得逃逸指针）。
pub fn encode_batch(events: &[Bytes]) -> Vec<u8> {
    // 预估容量：4(count) + 每条 4(len) + payload，单次分配避免 realloc（热路径）。
    let payload_total: usize = events.iter().map(|e| e.len()).sum();
    let mut buf = Vec::with_capacity(4 + events.len() * 4 + payload_total);

    // count 截断保护：理论上一个 step 不会有 >u32::MAX 条事件；超出按 u32 截断（不会发生）。
    let count = events.len() as u32;
    buf.extend_from_slice(&count.to_le_bytes());

    for ev in events {
        // 单条 >4GiB 不可能（DomainEvent JSON）；as u32 安全。
        let len = ev.len() as u32;
        buf.extend_from_slice(&len.to_le_bytes());
        buf.extend_from_slice(ev.as_ref());
    }
    buf
}

/// 解码（测试 / 对端参考实现）：还原成各条原文 slice。非法/截断 → None（零信任）。
///
/// 生产 C++ 侧按同算法实现；此函数主要给 FFI 集成测试断言回调 bytes 正确。
pub fn decode_batch(buf: &[u8]) -> Option<Vec<&[u8]>> {
    if buf.len() < 4 {
        return None;
    }
    let count = u32::from_le_bytes(buf[0..4].try_into().ok()?) as usize;
    let mut out = Vec::with_capacity(count);
    let mut off = 4usize;
    for _ in 0..count {
        if off + 4 > buf.len() {
            return None;
        }
        let len = u32::from_le_bytes(buf[off..off + 4].try_into().ok()?) as usize;
        off += 4;
        if off + len > buf.len() {
            return None;
        }
        out.push(&buf[off..off + len]);
        off += len;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_multi_event() {
        let events = vec![
            Bytes::from_static(b"{\"event\":\"a\"}"),
            Bytes::from_static(b"{\"event\":\"bb\"}"),
            Bytes::from_static(b""),
        ];
        let buf = encode_batch(&events);
        let decoded = decode_batch(&buf).expect("decode");
        assert_eq!(decoded.len(), 3);
        assert_eq!(decoded[0], b"{\"event\":\"a\"}");
        assert_eq!(decoded[1], b"{\"event\":\"bb\"}");
        assert_eq!(decoded[2], b"");
    }

    #[test]
    fn empty_batch_header_only() {
        let buf = encode_batch(&[]);
        assert_eq!(buf.len(), 4);
        assert_eq!(decode_batch(&buf).expect("decode").len(), 0);
    }

    #[test]
    fn decode_rejects_truncated() {
        assert!(decode_batch(&[]).is_none());
        assert!(decode_batch(&[1, 0, 0]).is_none()); // <4 bytes
                                                     // count=1 but no len follows
        assert!(decode_batch(&[1, 0, 0, 0]).is_none());
        // count=1, len=10 but only 2 payload bytes
        assert!(decode_batch(&[1, 0, 0, 0, 10, 0, 0, 0, b'x', b'y']).is_none());
    }
}
