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
/// **入参顺序无关**：内部先按 `(channel_id, from_seq)` 升序排序（ChannelId/Seq 均 Ord），
/// 再逐元素把 `channel_id` 的 26 字节 + `from_seq` 的 8 字节（大端）流入 FNV——确定性可复算
/// （HX-C010）。空集 → 返回 `None`（PingData.allHash 是 `Option`，无根群不带 hash）。
pub fn all_hash_fnv1a(cursors: &[RootCursor]) -> String {
    let mut sorted: Vec<RootCursor> = cursors.to_vec();
    // ChannelId: Ord（Id26 字节序）；同 channel 理论不重复，from_seq 作 tie-break 仍确定。
    sorted.sort_unstable_by(|a, b| {
        a.channel_id
            .cmp(&b.channel_id)
            .then(a.from_seq.0.cmp(&b.from_seq.0))
    });

    let mut h: u64 = 0xcbf2_9ce4_8422_2325; // FNV-1a 64bit offset basis（与 Id26::hash_u64 同基底）。
    for rc in &sorted {
        for &b in rc.channel_id.as_bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        for &b in &rc.from_seq.0.to_be_bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
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
}
