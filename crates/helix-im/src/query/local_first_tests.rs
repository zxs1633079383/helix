use super::*;

fn channel_id() -> ChannelId {
    ChannelId::from_str("ch00000000000000000000000a").unwrap()
}

fn request(limit: u32) -> MessageQueryRequest {
    MessageQueryRequest {
        channel_id: channel_id(),
        limit,
        window_token: "latest".to_string(),
    }
}

fn row(index: usize) -> Value {
    serde_json::json!({
        "id": format!("message-{index:03}"),
        "temporaryId": format!("temporary-{index:03}"),
        "createAt": index as i64,
    })
}

fn base64_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or_default() as u32;
        let b2 = chunk.get(2).copied().unwrap_or_default() as u32;
        let packed = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[((packed >> 18) & 63) as usize] as char);
        out.push(TABLE[((packed >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            TABLE[((packed >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[(packed & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

fn http_envelope(body: serde_json::Value) -> helix_core::tick::ReplyBytes {
    let raw = serde_json::to_vec(&body).expect("test receipt body serializes");
    let envelope = serde_json::json!({ "body": base64_encode(&raw) });
    helix_core::tick::ReplyBytes(bytes::Bytes::from(
        serde_json::to_vec(&envelope).expect("test HTTP envelope serializes"),
    ))
}

fn emitted_event(effect: &Effect) -> Option<serde_json::Value> {
    let Effect::Emit { event } = effect else {
        return None;
    };
    serde_json::from_slice(event.0.as_ref()).ok()
}

#[test]
fn disabled_maps_to_unsupported_without_inspecting_rows() {
    assert_eq!(
        classify_local_read(
            LocalStoreMode::Disabled,
            &request(20),
            &[row(1)],
            None,
            false,
        ),
        LocalReadCoverage::Unsupported
    );
}

#[test]
fn empty_rows_are_only_a_miss() {
    assert_eq!(
        classify_local_read(LocalStoreMode::Durable, &request(20), &[], None, false,),
        LocalReadCoverage::Miss
    );
}

#[test]
fn exact_remote_identity_window_is_complete_but_known_gap_revokes_it() {
    let mut rows = vec![row(2), row(1)];
    sort_recent_rows_desc(&mut rows);
    let proof = RecentMessageCoverage::from_remote(&rows, 2).unwrap();
    assert_eq!(
        classify_local_read(
            LocalStoreMode::Session,
            &request(2),
            &rows,
            Some(&proof),
            false,
        ),
        LocalReadCoverage::Complete
    );
    assert_eq!(
        classify_local_read(
            LocalStoreMode::Session,
            &request(2),
            &rows,
            Some(&proof),
            true,
        ),
        LocalReadCoverage::Partial
    );
}

#[test]
fn twenty_row_remote_window_never_proves_a_fifty_row_request() {
    let mut remote: Vec<Value> = (1..=20).map(row).collect();
    sort_recent_rows_desc(&mut remote);
    let proof = RecentMessageCoverage::from_remote(&remote, 20).unwrap();
    let mut local: Vec<Value> = (1..=50).map(row).collect();
    sort_recent_rows_desc(&mut local);
    assert_eq!(
        classify_local_read(
            LocalStoreMode::Durable,
            &request(50),
            &local,
            Some(&proof),
            false,
        ),
        LocalReadCoverage::Partial
    );
}

#[test]
fn fewer_than_twenty_remote_rows_can_prove_exhausted_larger_request() {
    let mut rows = vec![row(2), row(1)];
    sort_recent_rows_desc(&mut rows);
    let proof = RecentMessageCoverage::from_remote(&rows, 2).unwrap();
    assert_eq!(
        classify_local_read(
            LocalStoreMode::Durable,
            &request(50),
            &rows,
            Some(&proof),
            false,
        ),
        LocalReadCoverage::Complete
    );
}

#[test]
fn segment_expansion_above_twenty_does_not_prove_a_larger_window() {
    let mut rows: Vec<Value> = (1..=30).map(row).collect();
    sort_recent_rows_desc(&mut rows);
    let proof = RecentMessageCoverage::from_remote(&rows, 30).unwrap();
    assert_eq!(proof.remote_keys_desc.len(), REMOTE_RECENT_WINDOW);
    assert_eq!(
        classify_local_read(
            LocalStoreMode::Durable,
            &request(25),
            &rows,
            Some(&proof),
            false,
        ),
        LocalReadCoverage::Partial
    );
}

#[test]
fn exhausted_remote_window_drops_stale_server_rows_but_keeps_optimistic_rows() {
    let remote = vec![row(2)];
    let optimistic = serde_json::json!({
        "temporaryId": "optimistic-1",
        "createAt": 300,
    });
    let merged = merge_recent_rows(vec![optimistic.clone(), row(1)], remote, true, 20);
    assert_eq!(merged.len(), 2);
    assert!(merged.iter().any(|item| item == &optimistic));
    assert!(!merged.iter().any(|item| server_id(item) == "message-001"));
}

#[test]
fn latest_posts_parser_distinguishes_empty_success_from_failed_status() {
    let empty = parse_latest_posts_body(br#"{"status":"SUCCESS","data":[]}"#).unwrap();
    assert!(empty.is_empty());

    let error =
        parse_latest_posts_body(br#"{"status":"FAILED","message":"permission denied","data":[]}"#)
            .unwrap_err();
    assert!(error.to_string().contains("permission denied"));

    let missing_status = parse_latest_posts_body(br#"{"data":[]}"#).unwrap_err();
    assert!(missing_status.to_string().contains("missing string status"));
}

#[test]
fn local_first_terminal_emits_typed_timeline_frame_not_legacy_query_result() {
    let mut config = crate::module::ImConfig::default();
    config.auth_user_id = "viewer-1".to_string();
    let mut module = ImModule::new(config);
    let rows_desc = vec![serde_json::json!({
        "id": "message-1",
        "temporaryId": "temporary-1",
        "channelId": channel_id().as_str(),
        "userId": "author-1",
        "createAt": 10,
        "message": "hello",
        "type": "TEXT",
        "props": {},
    })];

    let mut request = request(20);
    request.window_token = "latest-query-1".to_string();
    let effect = module
        .emit_timeline_snapshot(&request, &rows_desc, 1_000)
        .expect("render-ready facts project to a typed frame");
    let Effect::Emit { event } = effect else {
        panic!("timeline query terminal must emit")
    };
    let envelope: serde_json::Value = serde_json::from_slice(event.0.as_ref()).unwrap();
    assert_eq!(envelope["event"], "im:interaction:frame");
    assert!(envelope["event"] != "im:messages:query_result");
    let frame = crate::interaction_frame::InteractionFrame::decode(
        &serde_json::to_vec(&envelope["data"]).unwrap(),
    )
    .expect("live timeline output must satisfy the public Frame contract");
    assert_eq!(frame.mutations.len(), 1);
    assert_eq!(
        frame.mutations[0].surface,
        crate::interaction_frame::SurfaceId::MessageTimeline
    );
    assert_eq!(
        frame.mutations[0].scope.canonical_key(),
        format!(
            "timeline|{}:{}|{}:{}",
            channel_id().as_str().len(),
            channel_id().as_str(),
            "latest-query-1".len(),
            "latest-query-1"
        )
    );
}

#[test]
fn remote_failure_emits_failed_timeline_frame_not_legacy_status_event() {
    let mut config = crate::module::ImConfig::default();
    config.auth_user_id = "viewer-1".to_string();
    let mut module = ImModule::new(config);

    let effect = module
        .emit_timeline_failed(&request(20), 1_000)
        .expect("failure state still projects to a typed frame");
    let Effect::Emit { event } = effect else {
        panic!("timeline query failure must emit a Frame")
    };
    let envelope: serde_json::Value = serde_json::from_slice(event.0.as_ref()).unwrap();
    assert_eq!(envelope["event"], "im:interaction:frame");
    assert_ne!(envelope["event"], "im:post:increment-failed");

    let frame = crate::interaction_frame::InteractionFrame::decode(
        &serde_json::to_vec(&envelope["data"]).unwrap(),
    )
    .expect("failed timeline output must satisfy the public Frame contract");
    let crate::interaction_frame::TypedSurfaceMutation::TimelineSnapshot { view } =
        &frame.mutations[0].mutation
    else {
        panic!("failure must stay a complete timeline snapshot")
    };
    assert_eq!(
        view.state,
        crate::interaction_frame::SurfaceLoadState::Failed
    );
    assert!(view.items.is_empty());
}

#[test]
fn view_receipt_action_uses_authority_and_only_matching_frame_confirms_completion() {
    use helix_core::Module;

    let mut config = crate::module::ImConfig::default();
    config.auth_user_id = "viewer-1".to_string();
    let mut module = ImModule::new(config);
    let rows_desc = vec![serde_json::json!({
        "id": "message-author-1",
        "temporaryId": "temporary-author-1",
        "channelId": channel_id().as_str(),
        "userId": "viewer-1",
        "createAt": 10,
        "message": "hello",
        "type": "TEXT",
        "props": {},
    })];
    let mut query = request(20);
    query.window_token = "action-window".to_string();
    let timeline_effect = module
        .emit_timeline_snapshot(&query, &rows_desc, 1_000)
        .expect("author timeline emits an action-bound Frame");
    let Effect::Emit { event } = timeline_effect else {
        panic!("timeline must emit a frame")
    };
    let timeline_event: serde_json::Value = serde_json::from_slice(event.0.as_ref()).unwrap();
    let timeline_frame = crate::interaction_frame::InteractionFrame::decode(
        &serde_json::to_vec(&timeline_event["data"]).unwrap(),
    )
    .unwrap();
    let mutation = &timeline_frame.mutations[0];
    let crate::interaction_frame::TypedSurfaceMutation::TimelineSnapshot { view } =
        &mutation.mutation
    else {
        panic!("timeline action must originate in a snapshot")
    };
    let binding = view.items[0]
        .actions
        .iter()
        .find(|action| action.kind == "viewReceipt")
        .expect("self-authored server message has receipt capability");
    assert!(binding.enabled);
    assert!(!binding.action_ref.contains("message-author-1"));

    let envelope = crate::interaction_frame::ActionInvocationEnvelope {
        schema_version: crate::interaction_frame::ACTION_INVOCATION_ENVELOPE_SCHEMA_VERSION,
        request_id: "receipt-request-1".to_string(),
        actor_id: "viewer-1".to_string(),
        surface: mutation.surface,
        scope: mutation.scope.clone(),
        surface_epoch: 1,
        observed_revision: mutation.revision,
        invocation: crate::interaction_frame::ActionInvocation {
            action_ref: binding.action_ref.clone(),
            input: std::collections::BTreeMap::new(),
        },
    };

    let mut sink = EffectSink::new();
    let mut wrong_actor = envelope.clone();
    wrong_actor.request_id = "receipt-wrong-actor".to_string();
    wrong_actor.actor_id = "viewer-2".to_string();
    module
        .handle(
            &helix_core::Tick::Command(
                crate::client_api::build_action_invocation_command(
                    &serde_json::to_vec(&wrong_actor).unwrap(),
                )
                .unwrap(),
            ),
            1_001,
            &mut sink,
        )
        .unwrap();
    assert!(sink
        .as_slice()
        .iter()
        .all(|effect| !matches!(effect, Effect::Http { .. })));
    let rejected = sink
        .as_slice()
        .iter()
        .filter_map(emitted_event)
        .find(|event| event["event"] == "im:interaction:action-receipt")
        .expect("actor mismatch gets an explicit receipt");
    assert_eq!(rejected["data"]["status"], "rejected");

    sink.clear();
    module
        .handle(
            &helix_core::Tick::Command(
                crate::client_api::build_action_invocation_command(
                    &serde_json::to_vec(&envelope).unwrap(),
                )
                .unwrap(),
            ),
            1_002,
            &mut sink,
        )
        .unwrap();
    let accepted = sink
        .as_slice()
        .iter()
        .filter_map(emitted_event)
        .find(|event| event["event"] == "im:interaction:action-receipt")
        .expect("accepted invoke publishes a receipt");
    assert_eq!(accepted["data"]["status"], "accepted");
    assert!(
        sink.as_slice().iter().all(|effect| {
            emitted_event(effect).is_none_or(|event| event["event"] != "im:interaction:frame")
        }),
        "accepted must not be treated as action completion before an authority Frame"
    );
    let corr = sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Http { corr, .. } => Some(*corr),
            _ => None,
        })
        .expect("accepted receipt action prepares canonical HTTP work");

    sink.clear();
    module
        .handle(
            &helix_core::Tick::Command(
                crate::client_api::build_action_invocation_command(
                    &serde_json::to_vec(&envelope).unwrap(),
                )
                .unwrap(),
            ),
            1_003,
            &mut sink,
        )
        .unwrap();
    assert!(sink
        .as_slice()
        .iter()
        .all(|effect| !matches!(effect, Effect::Http { .. })));
    let replay = sink
        .as_slice()
        .iter()
        .filter_map(emitted_event)
        .find(|event| event["event"] == "im:interaction:action-receipt")
        .expect("accepted requestId is replay-protected");
    assert_eq!(replay["data"]["status"], "rejected");

    sink.clear();
    module
        .handle(
            &helix_core::Tick::PortReply {
                corr,
                outcome: helix_core::tick::PortOutcome::Ok(http_envelope(serde_json::json!({
                    "status": "SUCCESS",
                    "data": [{
                        "postId": "message-author-1",
                        "authorUserId": "viewer-1",
                        "snapshotId": "snapshot-1",
                        "snapshotVersion": 1,
                        "memberIds": ["viewer-1", "reader-1"],
                        "readBits": "11"
                    }]
                }))),
            },
            1_004,
            &mut sink,
        )
        .unwrap();
    let authority_frame = sink
        .as_slice()
        .iter()
        .filter_map(emitted_event)
        .find(|event| event["event"] == "im:interaction:frame")
        .expect("canonical HTTP reply produces a typed Frame");
    let authority_frame = crate::interaction_frame::InteractionFrame::decode(
        &serde_json::to_vec(&authority_frame["data"]).unwrap(),
    )
    .unwrap();
    let confirmation = crate::projector::action::matching_frame_confirmation(
        &authority_frame,
        "receipt-request-1",
    )
    .unwrap()
    .expect("only the matching authority Frame can confirm the action");
    assert_eq!(
        confirmation.mutations[0].surface,
        crate::interaction_frame::SurfaceId::MessageParticipantReceipts
    );
}
