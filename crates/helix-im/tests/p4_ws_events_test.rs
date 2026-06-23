//! P4 WS 全量 19 事件 + parser 边界 + dead-action 清理回放测试。
//!
//! 回归锚: P4（19 事件全集路由 / event_seq 三级 fallback / 内存 post 不推 cursor /
//!         dead action 不实现 / change_channel_approval 无 Status 后缀 / cursor 锚 nextSeq B6）
//!
//! 覆盖维度（03-测试策略 P4 WS 行）：
//!   - 19 个 wire action 各 1 帧 → 经 registry 正常路由（**不冒 UnsupportedWsAction 硬错**）；
//!   - event_seq 三/四级 fallback 三路（data.event_seq → props.channel_event_seq → 顶层 → data.seq）；
//!   - 内存构造 post（缺 event_seq）不推 cursor（边界，复用 P5b bot #2/#32 同口径）；
//!   - dead action（post_urgent / post_edited / posted）no-op，零 Effect；
//!   - change_channel_approval wire 值无 Status 后缀（常量名带 Status 是后端陷阱）；
//!   - **cursor 锚 nextSeq 性质（B6）**：末条 event.seq < nextSeq（phantom 尾）→ cursor 推 nextSeq。

mod common;
use common::*;

use helix_core::tick::InboundBytes;
use helix_core::{Module, Tick};
use serde_json::json;

/// 构造任意 action 的 WS 帧。
fn ws_frame(value: serde_json::Value) -> Tick {
    Tick::Inbound(InboundBytes(serde_json::to_vec(&value).unwrap().into()))
}

/// 直接喂帧并返回 handle 的 Result（断言路由不报错）。
fn dispatch(
    h: &mut Harness,
    value: serde_json::Value,
    now: u64,
) -> Result<(), helix_core::CoreError> {
    let tick = ws_frame(value);
    h.sink.clear();
    h.module.handle(&tick, now, &mut h.sink)
}

// ─── ① 19 事件全集：每个 wire action 经 registry 正常路由（不冒 UnsupportedWsAction）────
//
// 真源 19 事件常量（full-map partials/5 §1）。`post`(#1) 与 bot/agent post(#19) 同 action 值。
// 断言：每个 action 喂一帧，dispatch 返回 Ok（已注册路由），**不**升 CoreError::ModuleError
// （UnsupportedWsAction）。带最小合法 data 避免 data_required 早退误判路由。
#[test]
fn all_19_wire_actions_route_through_registry_without_error() {
    let cid1 = cid(1);
    // 18 个不同 wire action（post 复用单值；19 计数含 bot post 同名变体）。
    let actions: &[&str] = &[
        "post",
        "posts_update",
        "post_update",
        "post_read",
        "post_pin",
        "post_schedule_created",
        "post_schedule_canceled",
        "increment_channel",
        "increment_channel_end",
        "channel_created",
        "channel_member_update",
        "channel_member_role_updated",
        "channel_close",
        "update_channel",
        "update_channel_notice",
        "update_channel_member_nickName",
        "change_channel_approval",
        "quit_company",
    ];
    assert_eq!(
        actions.len(),
        18,
        "18 个去重 wire action（post 与 bot post 同值，19 计数含变体）"
    );

    let mut h = Harness::new(&[1]);
    for action in actions {
        // 通用最小 data：带 channelId/id（兼容 channel 类用 id、post 类用 channelId）+ event_seq。
        let frame = json!({
            "v": 1,
            "action": action,
            "data": {
                "channelId": cid1, "id": cid1, "channel_id": cid1,
                "event_seq": 1,
                "userId": "u1", "userIds": ["u1"], "role": "member",
                "approvalStatus": true, "message": "m", "nickName": "n",
                "channels": [], "content": "c", "deleteAt": 0
            }
        });
        dispatch(&mut h, frame, 1_000).unwrap_or_else(|e| {
            panic!(
                "action `{action}` 须经 registry 正常路由（不冒 UnsupportedWsAction），got: {e:?}"
            )
        });
    }
}

// ─── ② 5 个 P4 新补 action 各单独验证 no-op（不破坏既有状态）─────────────────────
#[test]
fn p4_new_actions_are_graceful_noop() {
    let cid1 = cid(1);
    let new_actions: &[&str] = &[
        "post_pin",
        "channel_member_role_updated",
        "update_channel_notice",
        "change_channel_approval",
        "quit_company",
    ];
    let mut h = Harness::new(&[1]);
    let before = h.module.cursor_for(channel_id(1));

    for action in new_actions {
        let frame = json!({
            "v": 1, "action": action,
            "data": { "channelId": cid1, "id": cid1, "event_seq": 9, "approvalStatus": false }
        });
        dispatch(&mut h, frame, 1_000).expect("new action must route Ok");
        // 真源 default vec![]：无业务 Effect。
        assert!(
            h.sink.as_slice().is_empty(),
            "{action} 须 graceful no-op（无 Effect），got: {:?}",
            h.sink.as_slice()
        );
    }
    // cursor 未被这些 no-op action 推进。
    assert_eq!(
        h.module.cursor_for(channel_id(1)),
        before,
        "no-op action 不得推进 cursor"
    );
}

// ─── ③ change_channel_approval wire 值无 Status 后缀（常量名带 Status 是陷阱）──────
#[test]
fn change_channel_approval_action_value_has_no_status_suffix() {
    let cid1 = cid(1);
    let mut h = Harness::new(&[1]);

    // 正确 wire 值（无 Status）→ 路由命中（Ok）。
    dispatch(
        &mut h,
        json!({ "v": 1, "action": "change_channel_approval",
                "data": { "channelId": cid1, "approvalStatus": true, "message": "开启" } }),
        1_000,
    )
    .expect("`change_channel_approval`（无 Status 后缀）须命中注册");

    // 带 Status 后缀的错误形态 → 未注册 → UnsupportedWsAction（证伪：若误注册带后缀名则此处不报错）。
    let err = dispatch(
        &mut h,
        json!({ "v": 1, "action": "change_channel_approval_status",
                "data": { "channelId": cid1 } }),
        1_000,
    )
    .expect_err("带 Status 后缀的 action 值不存在 → 必须 UnsupportedWsAction（证伪注册名错误）");
    assert!(
        format!("{err:?}").contains("change_channel_approval_status"),
        "未支持的应是带 Status 后缀的错误形态，got: {err:?}"
    );
}

// ─── ④ dead action（posted / post_urgent / post_edited）no-op，零 Effect，不对账 ────
#[test]
fn dead_actions_are_noop_and_do_not_reconcile() {
    let cid1 = cid(1);
    let mut h = Harness::new(&[1]);
    // 先起一个在途 send（带 temporary_id）。
    let _ = h.step(send_message_cmd(1, "tmp_dead", "hi"), 1_000);
    let before_cursor = h.module.cursor_for(channel_id(1));

    for action in ["posted", "post_urgent", "post_edited"] {
        // 即便构造「最像 echo」的帧（带 temporary_id + 合法 id + event_seq）也 no-op。
        let frame = json!({
            "v": 1, "action": action,
            "data": { "id": sid(7), "temporary_id": "tmp_dead",
                      "channelId": cid1, "event_seq": 5, "message": "x" }
        });
        dispatch(&mut h, frame, 1_100).expect("dead action must route Ok (no-op)");
        assert!(
            h.sink.as_slice().is_empty(),
            "{action} 是 dead action → 零 Effect，got: {:?}",
            h.sink.as_slice()
        );
    }
    // pending send 不被对账（echo 真路径只走 post）。
    assert_eq!(
        h.module.pending_send_status("tmp_dead"),
        Some(helix_im::state::SendStatus::Local),
        "dead action posted 不得对账在途 send（echo 只走 post）"
    );
    // cursor 不被 dead action 推进。
    assert_eq!(
        h.module.cursor_for(channel_id(1)),
        before_cursor,
        "dead action 不得推进 cursor"
    );
}

// ─── ⑤ event_seq 三/四级 fallback：data.event_seq → props.channel_event_seq → 顶层 → data.seq ─
//
// 真源 parser::parse_inbound（legacy 数值帧的 event_seq 选取）。直接断言 parser 产出的
// EventEnvelope.seq（隔离 gate 行为：经 handle 走 channel gate 会按严格 +1 缓冲 gap，cursor
// 不直接 = parser 选取值，掩盖 fallback 真正选了哪级）。逐级缺字段验证下钻选取。

/// 经 parser 解析 legacy 帧，返回选取的 event_seq（u64）。
fn parsed_seq(value: serde_json::Value) -> u64 {
    let bytes = serde_json::to_vec(&value).unwrap();
    helix_im::parser::parse_inbound(&bytes)
        .expect("legacy frame parses")
        .seq
        .0
}

#[test]
fn event_seq_fallback_level1_data_event_seq_wins() {
    let cid1 = cid(1);
    // data.event_seq=7 必须压过 props.channel_event_seq=8 / 顶层=9 / data.seq=6。
    let seq = parsed_seq(json!({
        "event": "posted",
        "channel_event_seq": 9,
        "data": {
            "channel_id": cid1, "event_seq": 7, "seq": 6, "event_type": 1,
            "props": { "channel_event_seq": 8 }
        }
    }));
    assert_eq!(
        seq, 7,
        "level1: data.event_seq 必须胜出（压过 props/顶层/data.seq）"
    );
}

#[test]
fn event_seq_fallback_level2_props_channel_event_seq() {
    let cid1 = cid(1);
    // 缺 data.event_seq → 取 props.channel_event_seq=8（压过顶层 9 / data.seq 6）。
    let seq = parsed_seq(json!({
        "event": "posted",
        "channel_event_seq": 9,
        "data": {
            "channel_id": cid1, "seq": 6, "event_type": 1,
            "props": { "channel_event_seq": 8 }
        }
    }));
    assert_eq!(
        seq, 8,
        "level2: props.channel_event_seq 兜底（缺 data.event_seq 时）"
    );
}

#[test]
fn event_seq_fallback_level3_top_level_channel_event_seq() {
    let cid1 = cid(1);
    // 缺 data.event_seq + props → 取顶层 channel_event_seq=9（压过 data.seq 6）。
    let seq = parsed_seq(json!({
        "event": "posted",
        "channel_event_seq": 9,
        "data": { "channel_id": cid1, "seq": 6, "event_type": 1 }
    }));
    assert_eq!(
        seq, 9,
        "level3: 顶层 channel_event_seq 兜底（缺 data.event_seq + props 时）"
    );
}

#[test]
fn event_seq_fallback_level4_data_seq_last_resort() {
    let cid1 = cid(1);
    // 全缺 → 末级 data.seq=6（legacy 兼容兜底）。
    let seq = parsed_seq(json!({
        "event": "posted",
        "data": { "channel_id": cid1, "seq": 6, "event_type": 1 }
    }));
    assert_eq!(seq, 6, "level4: data.seq 末级兜底（全缺时）");
}

// ─── ⑥ 内存构造 post（缺 event_seq）不推 cursor（边界，复用 P5b bot #2/#32 口径）──────
#[test]
fn in_memory_post_without_event_seq_does_not_advance_cursor() {
    let cid1 = cid(1);
    let mut h = Harness::new(&[1]);
    let before = h.module.cursor_for(channel_id(1));

    // action="post" 但**缺 event_seq / props / 顶层 seq**（bot/agent 内存构造 post 的形态）：
    // parse_post_frame 硬 gate event_seq 缺失 → None → no-op，不喂 ch.ingest → cursor 不动。
    let frame = json!({
        "v": 1, "action": "post",
        "data": { "channelId": cid1, "id": sid(9), "message": "bot 内存构造提示" }
    });
    dispatch(&mut h, frame, 1_000).expect("post routes Ok");
    assert_eq!(
        h.module.cursor_for(channel_id(1)),
        before,
        "缺 event_seq 的内存构造 post 不得推进 cursor（边界零信任）"
    );
}

// ─── ⑦ cursor 锚 nextSeq 性质（B6）：末条 event.seq < nextSeq（phantom 尾）→ cursor 推 nextSeq ─
//
// 真源：服务端权威水位 nextSeq 可 > 实际下发 events 的 max（尾部 phantom seq 不进 events 数组）。
// commit_target = max(nextSeq, max(event.seq)) → cursor 必须推到 nextSeq，不能停在 max(event.seq)。
// 否则 cursor 落后服务端水位 → 后续 from_seq < 水位 → sync 风暴（HX-C008 / B6）。
#[test]
fn cursor_anchors_next_seq_not_max_event_seq() {
    let mut h = Harness::new(&[1]);
    h.start(0);
    h.step(
        Tick::Connected(helix_core::effect::TransportId::from_raw(0)),
        100,
    );
    h.step(hello_inbound("CONN_B6"), 200);

    // 触发一次 sync：用 increment + global-end 起 inflight sync（拿 corr）。
    h.module.ingest_increment(
        &helix_im::sync_session::IncrementChannel {
            channel_id: channel_id(1),
            last_event_seq: helix_im::state::Seq(100),
            need_sync: true,
            raw: bytes::Bytes::from("{}".to_string()),
        },
        &mut h.sink,
    );
    h.sink.clear();
    h.module.ingest_increment_end(None, &mut h.sink);

    // 取 sync/notify 的 corr。
    let corr = h
        .sink
        .as_slice()
        .iter()
        .find_map(|e| match e {
            helix_core::Effect::Http { corr, req } if req.url.ends_with("/channel/sync/notify") => {
                Some(corr.raw())
            }
            _ => None,
        })
        .expect("global-end 应发 sync/notify");

    // 喂 events 响应：实际 events 末条 seq=3，但 nextSeq=10（phantom 尾 seq 4..10 不在 events）。
    h.step(
        port_reply_with_body(
            corr,
            enveloped(sync_reply_events_with_next(1, &[1, 2, 3], 10)),
        ),
        2_000,
    );
    // sync 落库经 ChannelPersist corr 推进 cursor → 喂该 persist 的 Ok 回报。
    // persist corr = sync corr 后单调下一个（alloc 顺序：sync_corr 后是 persist_corr）。
    // 直接断言 cursor 经 commit_target=max(10, 3)=10 推进——需先让 persist Ok 到达。
    // 取本批 Persist 的 corr。
    let persist_corr = h
        .sink
        .as_slice()
        .iter()
        .find_map(|e| match e {
            helix_core::Effect::Persist { corr, .. } => Some(corr.raw()),
            _ => None,
        })
        .expect("events 响应应产出 Persist（落库）");
    h.step(port_reply_ok(persist_corr), 2_100);

    assert_eq!(
        h.module.cursor_for(channel_id(1)),
        Some(10),
        "cursor 必须锚 nextSeq=10（非 max(event.seq)=3）：尾部 phantom 不卡 cursor（B6/HX-C008）"
    );
}
