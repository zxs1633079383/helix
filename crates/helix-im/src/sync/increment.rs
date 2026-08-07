//! UC-4 离线同步增量——根群 cursor 快照 + allHash（心跳 gap 补偿 piggyback 的纯数据底座）。
//!
//! ## 为什么（接管 vs 现网）
//!
//! UC-4.4 心跳 gap 补偿：客户端 ping 帧 piggyback 全量根群 cursor 快照 + 它们的聚合 hash
//! （`PingData{cursors, allHash}`，types/sync.ts:242）。服务端比对本端权威水位，pong 回
//! `{gaps, hashMismatch}`（types/sync.ts:259）让客户端逐 channel 补偿 sync。本模块只产
//! **纯数据**（cursor 列表 + 确定性 hash），真 ping 帧组装在 `acl::to_effect::ping_frame`，
//! 真补偿调度在 sync_scheduler（HX-C001：core 零 I/O，im 只算定 Effect 数据）。
//!
//! ## 不变量
//!
//! - **确定性有序**（HX-C010）：`all_hash_fnv1a` 入参顺序无关——内部按 `ChannelId` 升序排序后
//!   再喂 FNV，杜绝 HashMap SipHash 随机迭代序导致同一组 cursor 算出不同 hash（→ 误判 hashMismatch
//!   → sync 风暴）。证伪锚点见 `uc4_sync_increment_test::all_hash_is_deterministic_and_order_independent`。
//! - **小写 16-hex**：FNV-1a 64bit → `{:016x}`（现网契约：allHash 16 位小写 hex）。
//! - **热路径友好**（HX-C005）：单次 hash = O(n log n) 排序（n=根群数，冷心跳路径）+ O(n) 流式
//!   FNV，零中间 String 分配（hash 直接累在 u64，末尾一次 format）。

use crate::state::{ChannelId, ImState, Seq};

/// UC-4.4 心跳 piggyback：全量已知 channel 的 cursor 快照，**确定性升序**（ChannelId:Ord 锚定，
/// HX-C010——`channels` 是 HashMap，迭代序逐进程 SipHash 随机，必须排序后再喂 allHash / 帧拼装，
/// 否则同一组 cursor 算出不同 allHash → 误判 hashMismatch → sync 风暴）。
///
/// 冷心跳路径（ping_interval 级），O(n log n) 排序零热路径代价（HX-C005）。自由函数（不挂 ImState
/// impl）以收窄 state.rs 体积（结构闸『只减不增』）——纯读 `state.channels`，无副作用。
pub fn heartbeat_root_cursors(state: &ImState) -> Vec<RootCursor> {
    let mut roots: Vec<RootCursor> = state
        .channels
        .iter()
        .filter(|(_, channel)| !channel.is_terminal())
        .map(|(&id, ch)| RootCursor {
            channel_id: id,
            from_seq: ch.cursor.value(),
        })
        .collect();
    roots.sort_unstable_by(|a, b| a.channel_id.cmp(&b.channel_id));
    roots
}

/// 一条根群 cursor 快照（心跳 piggyback / allHash 输入元素）。
///
/// 仅根群（root_id 空）的 per-channel max event_seq 进 allHash——话题群混入会污染聚合 hash
/// （现网 `set_root_cursor` 仅更新根群镜像，full-map §271）。话题/根群的甄别由调用方负责，
/// 本结构只承载已甄别的根群 cursor。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RootCursor {
    pub channel_id: ChannelId,
    pub from_seq: Seq,
}

/// 计算一组根群 cursor 的 allHash（FNV-1a 64bit → 16 位小写 hex）。
///
/// **byte-exact 契约（迁移单测必过）**：与现网 `all_hash.rs::compute_all_hash` /
/// Go `sync_reconcile.go::computeAllHash` **字节对齐**——不是把 raw id/seq 字节直接喂 FNV，
/// 而是先把每项格式化成 **`"<channelId>:<seq>"` 字符串**（channelId 用 26 字符 base32 文本、
/// seq 用十进制），按 channelId 升序、项间 `\n` join 成单串，整串 UTF-8 字节喂 FNV-1a。
/// 锚定向量：`{"a":5,"b":10}` → 字节流 `"a:5\nb:10"` → `87da68614226ef44`；空集 → offset
/// basis `cbf29ce484222325`。**若用 raw 字节形态 → 两端永久 hashMismatch → sync 风暴**
/// （Round-3 偏差修复，full-map/partials/8 §5.7 + §7.5）。
///
/// **入参顺序无关**（HX-C010）：内部按 `(channel_id, from_seq)` 升序排序后再拼串，杜绝
/// HashMap SipHash 随机迭代序导致同组 cursor 算出不同 hash。
pub fn all_hash_fnv1a(cursors: &[RootCursor]) -> String {
    let mut sorted: Vec<RootCursor> = cursors.to_vec();
    // ChannelId: Ord（Id26 字节序）；同 channel 理论不重复，from_seq 作 tie-break 仍确定。
    sorted.sort_unstable_by(|a, b| {
        a.channel_id
            .cmp(&b.channel_id)
            .then(a.from_seq.0.cmp(&b.from_seq.0))
    });

    // byte-exact：`"<channelId>:<seq>"` 项，项间 `\n` join，整串喂 FNV（现网 all_hash.rs 字节序）。
    let joined = sorted
        .iter()
        .map(|rc| format!("{}:{}", rc.channel_id.as_str(), rc.from_seq.0))
        .collect::<Vec<_>>()
        .join("\n");

    let mut h: u64 = 0xcbf2_9ce4_8422_2325; // FNV-1a 64bit offset basis（空串 → 原样返回）。
    for &b in joined.as_bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{h:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cid(c: char) -> ChannelId {
        ChannelId::from_str(&std::iter::repeat(c).take(26).collect::<String>()).unwrap()
    }

    #[test]
    fn empty_set_hash_is_offset_basis() {
        // 空集仍可算（offset basis 的 hex）——调用方按空 cursors 决定是否带 allHash。
        let h = all_hash_fnv1a(&[]);
        assert_eq!(h, format!("{:016x}", 0xcbf2_9ce4_8422_2325u64));
    }

    /// byte-exact 契约证伪锚（Round-3 偏差修复）：allHash **必须**是 `"<channelId>:<seq>"` 项
    /// `\n` join 后整串喂 FNV-1a，而非 raw 26B id + 8B BE seq 字节。本测试在测试内**独立**用
    /// 字符串形态复算同一向量，与 `all_hash_fnv1a` 比对——若实现回退成 raw-byte 形态，两值不等 FAIL。
    /// 现网锚定向量 `{"a":5,"b":10}→"a:5\nb:10"→87da68614226ef44` 因真 channelId 恒 26 字符
    /// 无法逐字复现，故改用真 26 字符 id 自洽复算锁定「字符串形态」这一不变量。
    #[test]
    fn all_hash_is_string_form_not_raw_bytes() {
        let cursors = [
            RootCursor {
                channel_id: cid('a'),
                from_seq: Seq(5),
            },
            RootCursor {
                channel_id: cid('b'),
                from_seq: Seq(10),
            },
        ];
        // 独立复算：字符串形态 `"<id>:<seq>"` join `\n`（与现网 all_hash.rs 同形态）。
        let joined = format!("{}:5\n{}:10", cid('a').as_str(), cid('b').as_str());
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for &byte in joined.as_bytes() {
            h ^= byte as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        let expected = format!("{h:016x}");
        assert_eq!(
            all_hash_fnv1a(&cursors),
            expected,
            "allHash 必须用 \"<channelId>:<seq>\" 字符串形态喂 FNV（现网 byte-exact 契约），非 raw 字节"
        );
    }

    /// FNV-1a 算法本体对锚定字节流 `"a:5\nb:10"` 的输出 = `87da68614226ef44`（现网迁移单测向量）。
    /// 锁住 FNV 常量 + 字节序正确（与 channelId 长度无关，纯算法层）。
    #[test]
    fn fnv1a_canonical_anchor_vector() {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for &byte in b"a:5\nb:10" {
            h ^= byte as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        assert_eq!(format!("{h:016x}"), "87da68614226ef44");
    }

    #[test]
    fn order_independent() {
        let a = vec![
            RootCursor {
                channel_id: cid('a'),
                from_seq: Seq(1),
            },
            RootCursor {
                channel_id: cid('b'),
                from_seq: Seq(2),
            },
        ];
        let mut b = a.clone();
        b.reverse();
        assert_eq!(all_hash_fnv1a(&a), all_hash_fnv1a(&b));
    }

    #[test]
    fn seq_change_flips_hash() {
        let a = [RootCursor {
            channel_id: cid('a'),
            from_seq: Seq(1),
        }];
        let b = [RootCursor {
            channel_id: cid('a'),
            from_seq: Seq(2),
        }];
        assert_ne!(all_hash_fnv1a(&a), all_hash_fnv1a(&b));
    }

    #[test]
    fn always_16_lower_hex() {
        let h = all_hash_fnv1a(&[RootCursor {
            channel_id: cid('z'),
            from_seq: Seq(0),
        }]);
        assert_eq!(h.len(), 16);
        assert!(h
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    }

    #[test]
    fn heartbeat_omits_terminal_channels() {
        let mut state = ImState::new();
        let active = cid('a');
        let closed = cid('b');
        state
            .channels
            .insert(active, crate::channel::Channel::new(active, 3));
        let mut closed_channel = crate::channel::Channel::new(closed, 4);
        closed_channel.mark_projection_terminal(Seq(0));
        state.channels.insert(closed, closed_channel);

        let roots = heartbeat_root_cursors(&state);
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].channel_id, active);
        assert_eq!(roots[0].from_seq, Seq(3));
    }
}
