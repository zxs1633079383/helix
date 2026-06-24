//! Transport Effect factories.

use bytes::Bytes;
use helix_core::effect::TransportId;
use helix_core::Effect;

/// 出站 WS ping 帧（带单调 `seq` + UC-4.4 心跳 gap 补偿 piggyback `PingData{cursors,allHash}`）。
///
/// 现网 Go `websocket_router.go:34 if r.Seq <= 0` 拒绝 seq≤0（"Invalid sequence" → 关连接），
/// 故每条出站 WS 帧必带 `seq = ++client_seq`。
///
/// **piggyback（UC-4.4 / PingData，types/sync.ts:242）**：`data.cursors` = 全量根群 cursor 快照
/// （`[{channelId, fromSeq}]`，确定性升序，HX-C010），`data.allHash` = 这组 cursor 的 FNV-1a 64bit
/// 16-hex（`sync_increment::all_hash_fnv1a`）。服务端比对水位 → pong 回 `{gaps, hashMismatch}` 触发
/// 逐 channel 补偿 sync。**空根群集**（无 cursor）→ `cursors:[]` 且**省 allHash**（`allHash?:` 可选，
/// 无根群不带 hash，避免空集 hash 误判 mismatch）。
///
/// `roots` 由 ping 时 `module` 从 `state.channels` 快照（已甄别根群），本函数纯组装（HX-C001）。
pub fn ping_frame(seq: u64, roots: &[crate::sync_increment::RootCursor]) -> Effect {
    let cursors: Vec<serde_json::Value> = roots
        .iter()
        .map(|rc| {
            serde_json::json!({ "channelId": rc.channel_id.as_str(), "fromSeq": rc.from_seq.0 })
        })
        .collect();
    let mut data = serde_json::json!({ "cursors": cursors });
    if !roots.is_empty() {
        data["allHash"] = serde_json::Value::String(crate::sync_increment::all_hash_fnv1a(roots));
    }
    let payload = serde_json::json!({ "action": "ping", "seq": seq, "data": data });
    let bytes = Bytes::from(
        serde_json::to_vec(&payload).expect("ping_frame: static JSON shape must not fail"),
    );
    Effect::Send {
        transport: TransportId::from_raw(0),
        frame: bytes,
    }
}
