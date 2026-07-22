//! HTTP-6 回归（dev-zlc / ADR-007 方向）：sync HTTP 回报是 ADR-007 信封
//! `{status,headers,body:base64(裸 Go body)}`；handle_sync_reply 必须**先解信封 + base64 decode**
//! 出裸 Go body 再喂 parse_sync_response。
//!
//! 回归锚: #6（离线 backfill 第二阻塞）。feat 在 driver 删信封返裸 body；dev-zlc 保 ADR-007 信封
//! （host run_http_envelope 双端共享，native+FFI parity），改在**消费侧**（module.rs handle_sync_reply）
//! 加解信封+内联 base64 decode 层。两端方向相反，本测试钉死 dev-zlc 方向的契约。
//!
//! ## Go-independent / 可证伪（双向断言）
//!
//! 不连真 Go：直接在 sync-pull corr 上灌信封字节 / 把同信封喂裸 parser，对比行为——
//! - **ADR-007 信封**（driver 2xx 实际回灌形态，common::sync_envelope 已包）经 handle_sync_reply
//!   解信封 + base64 decode → Persist；Persist Ok 后 Emit（>0）并推进 cursor。
//!   证「解信封修复真把数据放行」且不绕过 G-S2/G-S3 持久屏障。
//! - **同一信封若不解层、直喂 parse_sync_response** → status=200(int) 非 SUCCESS-string 跳过、
//!   无 data.entries → 静默 NoChange（0 persist / 0 emit）。证「#6 bug 现场：删掉解层即离线消息
//!   再次静默丢失」。两向一起钉死契约。

mod common;
use common::*;

/// 触发一次 gate sync，返回 sync-pull 的 Http corr（与 e3/s6 测试同构）。
fn drive_sync_pull(h: &mut Harness) -> u64 {
    // 大缺口（seq=99 / cursor=0）→ **立即** backfill resync（根治 fix/im-sync-cursor-catchup）：
    // sync Http 在 ingest 当步即发出，不再等 1s gate timer fire。
    let gate = h.step(inbound_event(1, 99, 1), 1000);
    gate.iter()
        .find_map(|e| match e {
            EffectKind::Http(c) => Some(*c),
            _ => None,
        })
        .expect("大缺口 ingest 当步即发 sync Http（立即 backfill）")
}

#[test]
fn http6_raw_go_body_drives_persist_and_emit() {
    let mut h = Harness::new_with_identity(&[1], "viewer-444", "tenant-1", "", "", "");
    let sync_corr = drive_sync_pull(&mut h);

    // 真 wire `events`+messages map 裸 Go body。用「可见」消息（msgId 在 messages map）→ 非 phantom
    // → handle_sync_reply 解信封 + base64 decode 后既落库又 emit post:received。
    let raw = sync_reply_events_with_messages(
        1,
        &[ev(1, 1, "msg_a"), ev(2, 1, "msg_b"), ev(3, 1, "msg_c")],
        &[
            ("msg_a", "hello a", ""),
            ("msg_b", "hello b", ""),
            ("msg_c", "hello c", ""),
        ],
        3,
    );
    // dev-zlc driver 把裸 body 包成 ADR-007 信封回灌；enveloped() 复刻该字节。
    let effects = h.step(port_reply_with_body(sync_corr, enveloped(raw)), 3000);

    let persist_count = effects
        .iter()
        .filter(|e| matches!(e, EffectKind::PersistAtomic(_)))
        .count();
    let emit_count_before_persist = effects
        .iter()
        .filter(|e| matches!(e, EffectKind::Emit))
        .count();

    assert!(
        persist_count > 0,
        "ADR-007 信封经解层后应产出 Persist（落库），got effects {effects:?}"
    );
    assert_eq!(
        emit_count_before_persist, 0,
        "ADR-007 解层后 mutation 仍须等待 Persist ack，got effects {effects:?}"
    );

    let persist_corr = effects
        .iter()
        .find_map(|effect| match effect {
            EffectKind::PersistAtomic(corr) => Some(*corr),
            _ => None,
        })
        .expect("ADR-007 解层后必须生成相关 Persist");
    let committed = h.step(
        port_reply_with_body(persist_corr, bytes::Bytes::new()),
        3_100,
    );
    assert!(
        committed
            .iter()
            .any(|effect| matches!(effect, EffectKind::Emit)),
        "Persist ack 后应释放 mutation 事件，got effects {committed:?}"
    );
}

#[test]
fn http6_legacy_envelope_is_silently_swallowed() {
    use helix_im::parser::parse_sync_response;
    use helix_im::sync_session::SyncResponse;

    // 同样 3 条可见事件的裸 Go body，包成 ADR-007 信封。证「#6 bug 现场」：若**不**解层、直把信封
    // 喂裸 parse_sync_response（= dev-zlc 修复前 / 删掉解信封后的行为）→ status=200(int) 非
    // SUCCESS-string 跳过、无 data.entries → 静默 NoChange。证明删掉解层即离线消息再次丢失。
    let raw = sync_reply_events_with_messages(
        1,
        &[ev(1, 1, "msg_a"), ev(2, 1, "msg_b"), ev(3, 1, "msg_c")],
        &[
            ("msg_a", "hello a", ""),
            ("msg_b", "hello b", ""),
            ("msg_c", "hello c", ""),
        ],
        3,
    );
    let envelope = enveloped(raw);

    // 裸 parser 吃信封：找不到 entries → NoChange（0 persist / 0 emit 的根因）。
    let response = parse_sync_response(envelope.as_ref(), channel_id(1))
        .expect("信封是合法 JSON，parser 不报错，只是找不到 entries");
    assert!(
        matches!(response, SyncResponse::NoChange),
        "信封未解层直喂 parser 必须静默 NoChange（0 persist / 0 emit 的 #6 bug 现场），got: {response:?}"
    );
}

/// 回归锚: BLOCKING（holistic-review 抓出）—— 防 `sync_unread_bump_ops` 在 `module.rs` SyncPull
/// 分支变回「函数在、零调用」死代码（离线未读 +N 落 DB 被静默丢）。本测试钉死 module.rs 的
/// `persist_ops.extend(sync_unread_bump_ops(..))` 调用点：删之 → 无 GuardedBump unread_count op
/// → FAIL（HX-C011 可证伪；per-phase byte-diff 漏掉的正是这种「消费侧 call-site 未接线」缺陷）。
/// viewer-444 不在消息 viewers 且不是 sender，因此本地可见性必须显式由 fixture 授予；本测试
/// 只对已进入 recovery authority 批的可见 type1 检查 unread bump。
#[test]
fn http6_offline_sync_visible_type1_bumps_unread_guardedbump() {
    use helix_core::effect::{Effect, StorageOp};

    let mut h = Harness::new_with_identity(&[1], "viewer-444", "tenant-1", "", "", "");
    let sync_corr = drive_sync_pull(&mut h);
    let raw = sync_reply_events_with_messages(
        1,
        &[ev(1, 1, "msg_a"), ev(2, 1, "msg_b"), ev(3, 1, "msg_c")],
        &[("msg_a", "a", ""), ("msg_b", "b", ""), ("msg_c", "c", "")],
        3,
    );
    // dev-zlc driver 把裸 body 包成 ADR-007 信封回灌；enveloped() 复刻该字节。
    h.step(port_reply_with_body(sync_corr, enveloped(raw)), 3000);

    // 离线可见 type1 → channel.unread_count GuardedBump +N（与消息落库同一 Persist 批，
    // unread 列写成功后 cursor 才推进——CAP-9 在线↔离线未读 parity）。
    let total_bump: i64 = h
        .sink
        .as_slice()
        .iter()
        .filter_map(|e| match e {
            Effect::PersistAtomic { ops, .. } => Some(
                ops.iter()
                    .filter_map(|op| match op {
                        StorageOp::GuardedBump(s) if s.bump_col == "unread_count" => {
                            Some(s.bump_delta)
                        }
                        _ => None,
                    })
                    .sum::<i64>(),
            ),
            _ => None,
        })
        .sum();

    assert!(
        total_bump >= 1,
        "离线 sync 可见 type1 须经 GuardedBump 给 channel.unread_count +N（防 module.rs SyncPull \
         分支把 sync_unread_bump_ops 退回死代码 = 离线未读丢失），got total_bump={total_bump}"
    );
}
