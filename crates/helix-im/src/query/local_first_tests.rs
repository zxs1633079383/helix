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

/// 历史 root 只有 topic relation 时也必须闭合 strict activity 字段，并清理内部幂等键。
#[test]
fn historical_topic_root_closes_with_authority_safe_activity_defaults() {
    let parent_channel_id = channel_id();
    let topic_channel_id = crate::state::test_channel_id(88);
    let mut dialogs = crate::dialog_frame::DialogListFrameState::default();
    let scope = crate::dialog_frame::DialogListScope::new("viewer", "company");
    let topic_dialog = crate::dialog_frame::row_from_dialog_projection(&serde_json::json!({
        "id": topic_channel_id.as_str(),
        "displayName": "历史话题",
        "rootPostId": "root-1"
    }))
    .expect("topic dialog projection closes");
    dialogs.snapshot(scope, vec![topic_dialog]);
    let mut shaped = serde_json::json!([{
        "msgId": "root-1",
        "createAt": 123,
        "topic": {
            "channelId": topic_channel_id.as_str(),
            "latestReplyId": "internal-only"
        }
    }]);

    close_topic_links(&mut shaped, parent_channel_id.as_str(), &dialogs);

    assert_eq!(shaped[0]["topic"]["replyCount"], 0);
    assert_eq!(shaped[0]["topic"]["lastActivityAt"], 123);
    assert!(shaped[0]["topic"].get("latestReplyId").is_none());
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
fn exhausted_remote_window_keeps_newer_committed_local_rows_during_replica_lag() {
    let remote = vec![row(2), row(1)];
    let newer_committed = row(3);

    let merged = merge_recent_rows(vec![newer_committed.clone()], remote.clone(), true, 20);

    assert!(merged.iter().any(|item| item == &newer_committed));
    assert!(
        stale_local_server_rows_delete_op(channel_id(), &[newer_committed], &remote).is_none(),
        "滞后的 latest endpoint 不能删除已由 sync 持久化的更新事实"
    );
}

#[test]
fn remote_latest_preserves_local_sequence_metadata_for_the_same_message() {
    let local = serde_json::json!({
        "id": "message-sequenced",
        "temporary_id": "temporary-sequenced",
        "channel_id": channel_id().as_str(),
        "props": r#"{"channel_event_seq":4,"local_only":"kept-locally"}"#,
    });
    let remote = serde_json::json!({
        "id": "message-sequenced",
        "temporaryId": "temporary-sequenced",
        "channelId": channel_id().as_str(),
        "userId": "author-1",
        "createAt": 10,
        "message": "remote authority",
        "type": "TEXT",
        "props": {"remote_only": true},
    });

    let (rows, ops) =
        visible_remote_rows_and_cache_ops(channel_id(), vec![remote], &[local], "viewer-1")
            .expect("same-identity remote row should remain cacheable");

    assert_eq!(data::post_event_seq(&rows[0]), Some(4));
    assert_eq!(rows[0]["props"]["remote_only"], true);
    assert_eq!(rows[0]["props"]["channel_event_seq"], 4);
    assert!(rows[0]["props"].get("local_only").is_none());

    let helix_core::effect::StorageOp::BatchUpsert(spec) = &ops[0] else {
        panic!("latest cache operation should be a message upsert");
    };
    let props = spec.rows[0]
        .iter()
        .find_map(|(column, value)| (column == "props").then_some(value))
        .expect("message cache row should persist props");
    let helix_core::effect::SqlValue::Text(props) = props else {
        panic!("message props should be stored as JSON text");
    };
    let props: Value = serde_json::from_str(&props).expect("cached props should be valid JSON");
    assert_eq!(props["remote_only"], true);
    assert_eq!(props["channel_event_seq"], 4);
}

#[test]
fn remote_sequence_metadata_remains_authoritative_over_local_cache() {
    let local = serde_json::json!({
        "id": "message-sequenced",
        "temporary_id": "temporary-sequenced",
        "props": r#"{"channel_event_seq":4}"#,
    });
    let remote = serde_json::json!({
        "id": "message-sequenced",
        "temporaryId": "temporary-sequenced",
        "channelId": channel_id().as_str(),
        "userId": "author-1",
        "createAt": 10,
        "message": "remote authority",
        "type": "TEXT",
        "eventSeq": 9,
        "props": {"channel_event_seq": 9},
    });

    let (rows, _) =
        visible_remote_rows_and_cache_ops(channel_id(), vec![remote], &[local], "viewer-1")
            .expect("remote sequence should remain authoritative");

    assert_eq!(data::post_event_seq(&rows[0]), Some(9));
    assert_eq!(rows[0]["props"]["channel_event_seq"], 9);
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
    config.default_api_base_url = "http://java.test".to_string();
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
        .emit_timeline_snapshot_with_causation(&request, &rows_desc, 1_000, None, None)
        .expect("render-ready facts project to a typed frame");
    let Effect::Emit { event } = effect else {
        panic!("timeline query terminal must emit")
    };
    let envelope: serde_json::Value = serde_json::from_slice(event.0.as_ref()).unwrap();
    assert_eq!(envelope["event"], "im:interaction:frame");
    assert!(envelope["event"] != "im:messages:query_result");
    let frame = crate::interaction_frame::InteractionFrameV2::decode(
        &serde_json::to_vec(&envelope["data"]).unwrap(),
    )
    .expect("live timeline output must satisfy the public Frame contract");
    assert_eq!(frame.mutations.len(), 1);
    assert_eq!(
        frame.mutations[0].mutation.surface,
        crate::interaction_frame::SurfaceId::MessageTimeline
    );
    assert_eq!(
        frame.mutations[0].mutation.scope.canonical_key(),
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
fn ordinary_authoritative_timeline_refresh_remains_a_revision_based_patch() {
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

    module
        .emit_timeline_snapshot_with_causation(&request(20), &rows_desc, 1_000, None, None)
        .expect("first authoritative fact establishes the timeline slot");
    let effect = module
        .emit_timeline_snapshot_with_causation(&request(20), &rows_desc, 1_001, None, None)
        .expect("ordinary authoritative refresh advances the existing slot");
    let Effect::Emit { event } = effect else {
        panic!("ordinary refresh must emit one typed Frame")
    };
    let envelope: serde_json::Value = serde_json::from_slice(event.0.as_ref()).unwrap();
    let mutation = &envelope["data"]["mutations"][0];
    assert_eq!(mutation["mutation"]["kind"], "timelinePatch");
    assert_eq!(mutation["baseRevision"], 1);
    assert_eq!(mutation["revision"], 2);
}

#[test]
fn full_authoritative_recent_window_exposes_load_older_page_contract() {
    let mut config = crate::module::ImConfig::default();
    config.auth_user_id = "viewer-1".to_string();
    config.default_api_base_url = "http://java.test".to_string();
    let mut module = ImModule::new(config);
    let mut rows_desc: Vec<Value> = (1..=REMOTE_RECENT_WINDOW)
        .map(|index| {
            serde_json::json!({
                "id": format!("message-{index:03}"),
                "temporaryId": format!("temporary-{index:03}"),
                "channelId": channel_id().as_str(),
                "userId": "author-1",
                "createAt": index as i64,
                "message": format!("message {index}"),
                "type": "TEXT",
                "props": {},
            })
        })
        .collect();
    sort_recent_rows_desc(&mut rows_desc);
    let coverage = RecentMessageCoverage::from_remote(&rows_desc, REMOTE_RECENT_WINDOW)
        .expect("full remote window establishes a non-exhausted proof");
    module
        .state
        .recent_message_coverage
        .insert(channel_id(), coverage);

    let effect = module
        .emit_timeline_snapshot_with_causation(&request(20), &rows_desc, 1_000, None, None)
        .expect("authoritative recent window projects");
    let Effect::Emit { event } = effect else {
        panic!("timeline projection must emit")
    };
    let envelope: Value = serde_json::from_slice(event.0.as_ref()).unwrap();
    let page = &envelope["data"]["mutations"][0]["mutation"]["view"]["page"];
    assert_eq!(page["hasOlder"], true);
    assert_eq!(page["hasMore"], true);
}

#[test]
fn attached_timeline_refreshes_after_a_durable_fact_even_before_first_snapshot() {
    let mut module = ImModule::new(crate::module::ImConfig::default());
    let scope = crate::interaction_frame::SurfaceScope::Timeline {
        channel_id: channel_id().as_str().to_string(),
        window_token: "latest".to_string(),
    };
    module.state.timeline_projector.register_attachment(&scope);
    assert!(module
        .state
        .timeline_projector
        .current_view(&scope)
        .is_none());

    let mut sink = helix_core::EffectSink::new();
    module
        .refresh_attached_latest_timeline(
            channel_id(),
            Some("send-request-before-snapshot".to_string()),
            &mut sink,
        )
        .expect("attached slot must schedule its authoritative local readback");

    assert!(sink
        .as_slice()
        .iter()
        .any(|effect| matches!(effect, Effect::Persist { .. })));
}

#[test]
fn transport_disconnect_preserves_attached_timeline_for_recovery_patch() {
    use helix_core::Module;

    let mut config = crate::module::ImConfig::default();
    config.auth_user_id = "viewer-1".to_string();
    let mut module = ImModule::new(config);
    let rows = vec![serde_json::json!({
        "id": "message-before-disconnect",
        "temporaryId": "temporary-before-disconnect",
        "channelId": channel_id().as_str(),
        "userId": "viewer-1",
        "createAt": 10,
        "message": "before disconnect",
        "type": "TEXT",
        "props": {},
    })];
    module
        .emit_timeline_snapshot_with_causation(&request(20), &rows, 1_000, None, None)
        .expect("initial attached timeline snapshot");
    let scope = crate::interaction_frame::SurfaceScope::Timeline {
        channel_id: channel_id().as_str().to_string(),
        window_token: "latest".to_string(),
    };

    let mut sink = EffectSink::new();
    module
        .handle(
            &helix_core::Tick::Disconnected(helix_core::effect::TransportId::from_raw(1)),
            1_001,
            &mut sink,
        )
        .expect("transport disconnect is a silent query-session boundary");
    assert!(sink.as_slice().is_empty());
    assert!(module.state.timeline_projector.is_attached(&scope));
    assert!(
        module
            .state
            .timeline_projector
            .current_view(&scope)
            .is_some(),
        "same-identity renderer revision remains mounted across a socket reconnect"
    );

    module
        .refresh_attached_latest_timeline(
            channel_id(),
            Some("recovery-persist-1".to_string()),
            &mut sink,
        )
        .expect("a recovered durable fact must re-enter the attached timeline");
    assert!(
        sink.as_slice()
            .iter()
            .any(|effect| matches!(effect, Effect::Persist { .. })),
        "recovery must schedule authoritative local readback instead of dropping the V2 frame"
    );
}

#[test]
fn durable_fact_refresh_uses_local_rows_without_get_latest_post_fallback() {
    use helix_core::Module;

    let mut config = crate::module::ImConfig::default();
    config.auth_user_id = "viewer-1".to_string();
    let mut module = ImModule::new(config);
    let scope = crate::interaction_frame::SurfaceScope::Timeline {
        channel_id: channel_id().as_str().to_string(),
        window_token: "latest".to_string(),
    };
    module.state.timeline_projector.register_attachment(&scope);

    let mut sink = EffectSink::new();
    module
        .refresh_attached_latest_timeline(
            channel_id(),
            Some("durable-send-refresh".to_string()),
            &mut sink,
        )
        .expect("durable fact schedules one local readback");
    let refresh_corr = sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Persist { corr, .. } => Some(*corr),
            _ => None,
        })
        .expect("durable refresh starts with a local message scan");
    let local_rows = vec![serde_json::json!({
        "id": "message-local",
        "temporaryId": "temporary-local",
        "channelId": channel_id().as_str(),
        "userId": "viewer-1",
        "createAt": 10,
        "message": "local durable fact",
        "type": "TEXT",
        "sendStatus": "sending",
        "props": {},
    })];

    sink.clear();
    module
        .handle(
            &helix_core::Tick::PortReply {
                corr: refresh_corr,
                outcome: helix_core::tick::PortOutcome::Ok(helix_core::tick::ReplyBytes(
                    bytes::Bytes::from(serde_json::to_vec(&local_rows).unwrap()),
                )),
            },
            1_001,
            &mut sink,
        )
        .expect("durable local rows project directly");

    assert!(
        sink.as_slice()
            .iter()
            .all(|effect| !matches!(effect, Effect::Http { .. })),
        "a confirmed local write must never trigger getLatestPost"
    );
    assert!(
        sink.as_slice()
            .iter()
            .filter_map(emitted_event)
            .any(|event| event["event"] == "im:interaction:frame"),
        "the local readback must release the attached timeline Frame"
    );
}

#[test]
fn optimistic_send_persist_refreshes_an_attached_timeline_as_a_causally_linked_frame() {
    use helix_core::Module;

    let mut config = crate::module::ImConfig::default();
    config.auth_user_id = "viewer-1".to_string();
    config.company_id = "team-1".to_string();
    let mut module = ImModule::new(config);
    let initial_rows = vec![serde_json::json!({
        "id": "message-before",
        "temporaryId": "temporary-before",
        "channelId": channel_id().as_str(),
        "userId": "viewer-1",
        "createAt": 10,
        "message": "before",
        "type": "TEXT",
        "props": {},
    })];
    module
        .emit_timeline_snapshot_with_causation(&request(50), &initial_rows, 1_000, None, None)
        .expect("the renderer attaches the latest timeline before sending");

    let mut sink = EffectSink::new();
    module
        .handle(
            &helix_core::Tick::Command(helix_core::tick::AppCommand::new(
                "im_send_message",
                serde_json::to_vec(&serde_json::json!({
                    "channel_id": channel_id().as_str(),
                    "text": "new message",
                    "req_id": "send-request-1",
                }))
                .unwrap(),
            )),
            1_001,
            &mut sink,
        )
        .expect("the send command starts the ordinary optimistic chain");
    let optimistic_corr = sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Persist { corr, .. } => Some(*corr),
            _ => None,
        })
        .expect("send must persist its optimistic row before HTTP settles");
    assert!(
        sink.as_slice().iter().all(|effect| !matches!(effect, Effect::Http { .. })),
        "ordinary text must not start posts/create before P1 has produced the authoritative timeline Frame"
    );
    let temporary_id = module
        .state
        .pending_sends
        .keys()
        .next()
        .expect("send owns one pending temporary id")
        .0
        .clone();
    let mut refreshed_rows = vec![
        serde_json::json!({
            "id": "message-before",
            "temporaryId": "temporary-before",
            "channelId": channel_id().as_str(),
            "userId": "viewer-1",
            "createAt": 10,
            "message": "before",
            "type": "TEXT",
            "props": {},
        }),
        serde_json::json!({
            "id": temporary_id,
            "temporaryId": temporary_id,
            "channelId": channel_id().as_str(),
            "userId": "viewer-1",
            "createAt": 11,
            "message": "new message",
            "type": "TEXT",
            "sendStatus": "sending",
            "props": {},
        }),
    ];
    sort_recent_rows_desc(&mut refreshed_rows);

    sink.clear();
    module
        .handle(
            &helix_core::Tick::PortReply {
                corr: optimistic_corr,
                outcome: helix_core::tick::PortOutcome::Ok(helix_core::tick::ReplyBytes(
                    bytes::Bytes::new(),
                )),
            },
            1_002,
            &mut sink,
        )
        .expect("P1 acknowledgement schedules a local-first projector refresh");
    let refresh_corr = sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Persist { corr, .. } => Some(*corr),
            _ => None,
        })
        .expect("an attached timeline refresh must read authoritative local rows");

    sink.clear();
    module
        .handle(
            &helix_core::Tick::PortReply {
                corr: refresh_corr,
                outcome: helix_core::tick::PortOutcome::Ok(helix_core::tick::ReplyBytes(
                    bytes::Bytes::from(serde_json::to_vec(&refreshed_rows).unwrap()),
                )),
            },
            1_003,
            &mut sink,
        )
        .expect("the confirmed local rows project without a legacy UI reducer");
    let frame_event = sink
        .as_slice()
        .iter()
        .filter_map(emitted_event)
        .find(|event| event["event"] == "im:interaction:frame")
        .expect("the attached timeline receives a typed Frame patch");
    let frame = crate::interaction_frame::InteractionFrameV2::decode(
        &serde_json::to_vec(&frame_event["data"]).unwrap(),
    )
    .expect("the refresh output satisfies the public InteractionFrame contract");
    let mutation = frame
        .mutations
        .iter()
        .find(|mutation| {
            mutation.mutation.surface == crate::interaction_frame::SurfaceId::MessageTimeline
        })
        .expect("the Frame includes the timeline surface");
    assert_eq!(
        mutation.mutation.causation_id.as_deref(),
        Some("send-request-1")
    );
    assert!(matches!(
        mutation.mutation.mutation,
        crate::interaction_frame::TypedSurfaceMutation::TimelinePatch { .. }
    ));
    let frame_index = sink
        .as_slice()
        .iter()
        .position(|effect| matches!(effect, Effect::Emit { .. }))
        .expect("P1 local readback emits its typed Frame");
    let http_index = sink
        .as_slice()
        .iter()
        .position(|effect| {
            matches!(
                effect,
                Effect::Http { req, .. } if req.url.ends_with("/posts/create")
            )
        })
        .expect("ordinary text starts posts/create after the P1 Frame");
    assert_eq!(
        sink.as_slice()
            .iter()
            .filter(|effect| matches!(effect, Effect::Http { .. }))
            .count(),
        1,
        "P1 readback must not add getLatestPost beside posts/create"
    );
    assert!(
        frame_index < http_index,
        "the durable sending Frame must leave Helix before posts/create can settle it"
    );
}

#[test]
fn optimistic_send_without_an_attached_timeline_releases_http_after_p1() {
    use helix_core::Module;

    let mut config = crate::module::ImConfig::default();
    config.auth_user_id = "viewer-1".to_string();
    config.company_id = "team-1".to_string();
    let mut module = ImModule::new(config);
    let mut sink = EffectSink::new();
    module
        .handle(
            &helix_core::Tick::Command(helix_core::tick::AppCommand::new(
                "im_send_message",
                serde_json::to_vec(&serde_json::json!({
                    "channel_id": channel_id().as_str(),
                    "text": "unattached message",
                    "req_id": "send-request-unattached",
                }))
                .unwrap(),
            )),
            1_001,
            &mut sink,
        )
        .expect("the ordinary send starts its P1 persist");
    let optimistic_corr = sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Persist { corr, .. } => Some(*corr),
            _ => None,
        })
        .expect("ordinary text has one P1 correlation");
    assert!(
        sink.as_slice()
            .iter()
            .all(|effect| !matches!(effect, Effect::Http { .. })),
        "HTTP still waits for the P1 acknowledgement"
    );

    sink.clear();
    module
        .handle(
            &helix_core::Tick::PortReply {
                corr: optimistic_corr,
                outcome: helix_core::tick::PortOutcome::Ok(helix_core::tick::ReplyBytes(
                    bytes::Bytes::new(),
                )),
            },
            1_002,
            &mut sink,
        )
        .expect("P1 acknowledgement does not require a detached surface readback");
    assert!(
        sink.as_slice()
            .iter()
            .any(|effect| matches!(effect, Effect::Http { .. })),
        "a detached renderer cannot block the durable ordinary send from reaching posts/create"
    );
}

#[test]
fn http_send_settlement_waits_for_its_durable_receipt_before_refreshing_the_attached_timeline() {
    use helix_core::Module;

    let mut config = crate::module::ImConfig::default();
    config.auth_user_id = "viewer-1".to_string();
    config.company_id = "team-1".to_string();
    let mut module = ImModule::new(config);
    let initial_rows = vec![serde_json::json!({
        "id": "message-before",
        "temporaryId": "temporary-before",
        "channelId": channel_id().as_str(),
        "userId": "viewer-1",
        "createAt": 10,
        "message": "before",
        "type": "TEXT",
        "props": {},
    })];
    module
        .emit_timeline_snapshot_with_causation(&request(50), &initial_rows, 2_000, None, None)
        .expect("the timeline is attached before the HTTP send settles");

    let mut sink = EffectSink::new();
    module
        .handle(
            &helix_core::Tick::Command(helix_core::tick::AppCommand::new(
                "im_send_message",
                serde_json::to_vec(&serde_json::json!({
                    "channel_id": channel_id().as_str(),
                    "text": "settled message",
                    "req_id": "send-request-2",
                }))
                .unwrap(),
            )),
            2_001,
            &mut sink,
        )
        .expect("the ordinary send chain starts");
    let optimistic_corr = sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Persist { corr, .. } => Some(*corr),
            _ => None,
        })
        .expect("the send chain starts with its P1 optimistic persist");
    assert!(
        sink.as_slice()
            .iter()
            .all(|effect| !matches!(effect, Effect::Http { .. })),
        "ordinary text HTTP waits for its P1 timeline Frame"
    );
    let temporary_id = module
        .state
        .pending_sends
        .keys()
        .next()
        .expect("send owns one pending temporary id")
        .0
        .clone();
    let mut settled_rows = vec![
        serde_json::json!({
            "id": "message-before",
            "temporaryId": "temporary-before",
            "channelId": channel_id().as_str(),
            "userId": "viewer-1",
            "createAt": 10,
            "message": "before",
            "type": "TEXT",
            "props": {},
        }),
        serde_json::json!({
            "id": temporary_id,
            "temporaryId": temporary_id,
            "channelId": channel_id().as_str(),
            "userId": "viewer-1",
            "createAt": 11,
            "message": "settled message",
            "type": "TEXT",
            "sendStatus": "sent",
            "props": {},
        }),
    ];
    sort_recent_rows_desc(&mut settled_rows);
    let mut sending_rows = settled_rows.clone();
    sending_rows
        .iter_mut()
        .find(|row| row["temporaryId"].as_str() == Some(temporary_id.as_str()))
        .expect("the optimistic row remains in the readback window")["sendStatus"] =
        serde_json::json!("sending");

    sink.clear();
    module
        .handle(
            &helix_core::Tick::PortReply {
                corr: optimistic_corr,
                outcome: helix_core::tick::PortOutcome::Ok(helix_core::tick::ReplyBytes(
                    bytes::Bytes::new(),
                )),
            },
            2_002,
            &mut sink,
        )
        .expect("P1 acknowledgement schedules its authoritative local readback");
    let optimistic_refresh_corr = sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Persist { corr, .. } => Some(*corr),
            _ => None,
        })
        .expect("P1 must read back its durable sending row before HTTP");

    sink.clear();
    module
        .handle(
            &helix_core::Tick::PortReply {
                corr: optimistic_refresh_corr,
                outcome: helix_core::tick::PortOutcome::Ok(helix_core::tick::ReplyBytes(
                    bytes::Bytes::from(serde_json::to_vec(&sending_rows).unwrap()),
                )),
            },
            2_003,
            &mut sink,
        )
        .expect("P1 readback emits a sending Frame before posts/create");
    let http_corr = sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Http { corr, req } if req.url.ends_with("/posts/create") => Some(*corr),
            _ => None,
        })
        .expect("the ordinary send chain owns one posts/create HTTP correlation after P1 Frame");

    sink.clear();
    module
        .handle(
            &helix_core::Tick::PortReply {
                corr: http_corr,
                outcome: helix_core::tick::PortOutcome::Ok(http_envelope(serde_json::json!({
                    "status": "SUCCESS"
                }))),
            },
            2_004,
            &mut sink,
        )
        .expect("HTTP settlement creates a durable terminal write");
    let terminal_corr = sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Persist { corr, .. } => Some(*corr),
            _ => None,
        })
        .expect("sent state must be acknowledged by local storage first");
    assert!(
        sink.as_slice()
            .iter()
            .all(|effect| !matches!(effect, Effect::Emit { .. })),
        "HTTP accepted is not itself a renderer confirmation"
    );

    sink.clear();
    module
        .handle(
            &helix_core::Tick::PortReply {
                corr: terminal_corr,
                outcome: helix_core::tick::PortOutcome::Ok(helix_core::tick::ReplyBytes(
                    bytes::Bytes::new(),
                )),
            },
            2_005,
            &mut sink,
        )
        .expect("terminal durable acknowledgement schedules the projector readback");
    let refresh_corr = sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Persist { corr, .. } => Some(*corr),
            _ => None,
        })
        .expect("the terminal receipt re-enters local-first rather than raw event rendering");

    sink.clear();
    module
        .handle(
            &helix_core::Tick::PortReply {
                corr: refresh_corr,
                outcome: helix_core::tick::PortOutcome::Ok(helix_core::tick::ReplyBytes(
                    bytes::Bytes::from(serde_json::to_vec(&settled_rows).unwrap()),
                )),
            },
            2_006,
            &mut sink,
        )
        .expect("the durable sent row projects to the attached timeline");
    let frame_event = sink
        .as_slice()
        .iter()
        .filter_map(emitted_event)
        .find(|event| event["event"] == "im:interaction:frame")
        .expect("only the readback releases the typed timeline patch");
    let frame = crate::interaction_frame::InteractionFrameV2::decode(
        &serde_json::to_vec(&frame_event["data"]).unwrap(),
    )
    .expect("the terminal projection satisfies the Frame contract");
    let mutation = frame
        .mutations
        .iter()
        .find(|mutation| {
            mutation.mutation.surface == crate::interaction_frame::SurfaceId::MessageTimeline
        })
        .expect("the terminal Frame patches the timeline");
    assert_eq!(
        mutation.mutation.causation_id.as_deref(),
        Some("send-request-2")
    );
    assert!(matches!(
        mutation.mutation.mutation,
        crate::interaction_frame::TypedSurfaceMutation::TimelinePatch { .. }
    ));
}

#[test]
fn remote_failure_emits_failed_timeline_frame_not_legacy_status_event() {
    let mut config = crate::module::ImConfig::default();
    config.auth_user_id = "viewer-1".to_string();
    let mut module = ImModule::new(config);

    let effect = module
        .emit_timeline_failed(&request(20), 1_000, None)
        .expect("failure state still projects to a typed frame");
    let Effect::Emit { event } = effect else {
        panic!("timeline query failure must emit a Frame")
    };
    let envelope: serde_json::Value = serde_json::from_slice(event.0.as_ref()).unwrap();
    assert_eq!(envelope["event"], "im:interaction:frame");
    assert_ne!(envelope["event"], "im:post:increment-failed");

    let frame = crate::interaction_frame::InteractionFrameV2::decode(
        &serde_json::to_vec(&envelope["data"]).unwrap(),
    )
    .expect("failed timeline output must satisfy the public Frame contract");
    let crate::interaction_frame::TypedSurfaceMutation::TimelineSnapshot { view } =
        &frame.mutations[0].mutation.mutation
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
fn non_query_timeline_patch_regrants_same_action_ref_at_new_revision() {
    use helix_core::Module;

    let mut config = crate::module::ImConfig::default();
    config.auth_user_id = "viewer-1".to_string();
    config.api_base_url = "http://go.test".to_string();
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
    query.window_token = "receipt-patch-window".to_string();
    let Effect::Emit { event } = module
        .emit_timeline_snapshot_with_causation(&query, &rows_desc, 1_000, None, None)
        .expect("initial query frame grants timeline actions")
    else {
        panic!("timeline query must emit a frame")
    };
    let event: serde_json::Value = serde_json::from_slice(event.0.as_ref()).unwrap();
    let initial_frame = crate::interaction_frame::InteractionFrameV2::decode(
        &serde_json::to_vec(&event["data"]).unwrap(),
    )
    .unwrap();
    let initial_mutation = &initial_frame.mutations[0].mutation;
    let crate::interaction_frame::TypedSurfaceMutation::TimelineSnapshot { view } =
        &initial_mutation.mutation
    else {
        panic!("initial timeline must be a snapshot")
    };
    let initial_binding = view.items[0]
        .actions
        .iter()
        .find(|action| action.kind == "viewReceipt")
        .expect("self-authored message has a receipt action")
        .clone();

    let mutations = module
        .state
        .timeline_projector
        .patch_receipts(&[serde_json::json!({
            "postId": "message-author-1",
            "readCount": 1,
        })])
        .expect("receipt authority advances the attached timeline");
    assert_eq!(mutations.len(), 1);
    let patch_revision = mutations[0].revision;
    let patch_scope = mutations[0].scope.clone();
    let crate::interaction_frame::TypedSurfaceMutation::TimelinePatch { patch } =
        &mutations[0].mutation
    else {
        panic!("receipt authority must produce a timeline patch")
    };
    let patch_binding = patch.view.items[0]
        .actions
        .iter()
        .find(|action| action.kind == "viewReceipt")
        .expect("receipt patch preserves the visible receipt action");
    assert_eq!(patch_binding.action_ref, initial_binding.action_ref);
    assert_ne!(patch_revision, initial_mutation.revision);

    let effect = module
        .build_registered_timeline_frame("timeline-receipts-test".to_string(), mutations, 1_001)
        .expect("non-query patch regrants bindings before frame emission");
    assert!(matches!(effect, Effect::Emit { .. }));

    let envelope = crate::interaction_frame::ActionInvocationEnvelope {
        schema_version: crate::interaction_frame::ACTION_INVOCATION_ENVELOPE_SCHEMA_VERSION,
        request_id: "receipt-after-patch".to_string(),
        actor_id: "viewer-1".to_string(),
        surface: crate::interaction_frame::SurfaceId::MessageTimeline,
        scope: patch_scope,
        surface_epoch: initial_binding.surface_epoch,
        observed_revision: patch_revision,
        invocation: crate::interaction_frame::ActionInvocation {
            action_ref: initial_binding.action_ref,
            input: std::collections::BTreeMap::new(),
        },
    };
    let mut sink = EffectSink::new();
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
        .expect("the visible patch revision authorizes the unchanged action ref");
    assert!(sink
        .as_slice()
        .iter()
        .any(|effect| matches!(effect, Effect::Http { .. })));
}

#[test]
fn retry_action_invocation_runs_canonical_media_retry_after_acceptance_and_releases_on_frame() {
    use helix_core::Module;

    let mut config = crate::module::ImConfig::default();
    config.auth_user_id = "viewer-1".to_string();
    config.default_api_base_url = "http://java.test".to_string();
    let mut module = ImModule::new(config);
    let channel_id = channel_id();
    let temporary_id = crate::state::TemporaryId("tmp-action-retry-file".to_string());
    let mut pending = crate::pending_send::PendingSend::new(
        temporary_id.clone(),
        helix_core::TimerId::from_raw(91),
        None,
    );
    pending.status = crate::state::SendStatus::UnSend;
    pending.upload_failed = true;
    pending.remaining_uploads = 1;
    pending.body = Some(serde_json::json!({
        "temporaryId": temporary_id.0,
        "channelId": channel_id.as_str(),
        "type": "FILE",
        "message": "",
        "props": {
            "file": {
                "name": "report.pdf",
                "upload": {"status": "failed"}
            }
        }
    }));
    module
        .state
        .pending_sends
        .insert(temporary_id.clone(), pending);
    module.state.failed_media_ops.insert(
        (
            temporary_id.clone(),
            crate::send::upload_props::UploadTarget::File,
        ),
        crate::send::upload_props::FailedMediaOp::Prepare(
            crate::send::upload_props::PendingMediaPrepare {
                temporary_id: temporary_id.clone(),
                channel_id,
                target: crate::send::upload_props::UploadTarget::File,
                input: crate::send::upload_props::MediaInput {
                    local_path: "/tmp/report.pdf".to_string(),
                    file_name: "report.pdf".to_string(),
                    content_type: "application/pdf".to_string(),
                    size: 9,
                    sha256: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                        .to_string(),
                },
            },
        ),
    );

    let rows_desc = vec![serde_json::json!({
        "temporary_id": temporary_id.0,
        "channel_id": channel_id.as_str(),
        "user_id": "viewer-1",
        "create_at": 10,
        "message": "",
        "type": "FILE",
        "props": {
            "file": {
                "name": "report.pdf",
                "upload": {"status": "failed"}
            }
        },
        "send_status": "unsend",
        "upload_progress_percent": 0
    })];
    let mut query = request(20);
    query.window_token = "retry-action-window".to_string();
    let Effect::Emit { event } = module
        .emit_timeline_snapshot_with_causation(&query, &rows_desc, 1_000, None, None)
        .expect("failed media timeline grants retry")
    else {
        panic!("failed media query must emit a timeline frame")
    };
    let event: serde_json::Value = serde_json::from_slice(event.0.as_ref()).unwrap();
    let frame = crate::interaction_frame::InteractionFrameV2::decode(
        &serde_json::to_vec(&event["data"]).unwrap(),
    )
    .unwrap();
    let mutation = &frame.mutations[0];
    let crate::interaction_frame::TypedSurfaceMutation::TimelineSnapshot { view } =
        &mutation.mutation.mutation
    else {
        panic!("initial retry timeline must be a snapshot")
    };
    let binding = view.items[0]
        .actions
        .iter()
        .find(|action| action.kind == "retry")
        .expect("failed media exposes retry capability")
        .clone();
    assert!(binding.enabled);
    assert!(!binding.action_ref.contains(temporary_id.0.as_str()));

    let envelope = crate::interaction_frame::ActionInvocationEnvelope {
        schema_version: crate::interaction_frame::ACTION_INVOCATION_ENVELOPE_SCHEMA_VERSION,
        request_id: "retry-action-request".to_string(),
        actor_id: "viewer-1".to_string(),
        surface: mutation.mutation.surface,
        scope: mutation.mutation.scope.clone(),
        surface_epoch: binding.surface_epoch,
        observed_revision: mutation.mutation.revision,
        invocation: crate::interaction_frame::ActionInvocation {
            action_ref: binding.action_ref.clone(),
            input: std::collections::BTreeMap::new(),
        },
    };

    let mut invalid = envelope.clone();
    invalid.request_id = "retry-action-invalid-input".to_string();
    invalid.invocation.input.insert(
        "unexpected".to_string(),
        crate::interaction_frame::SurfaceDataVm::String("value".to_string()),
    );
    let mut sink = EffectSink::new();
    module
        .handle(
            &helix_core::Tick::Command(
                crate::client_api::build_action_invocation_command(
                    &serde_json::to_vec(&invalid).unwrap(),
                )
                .unwrap(),
            ),
            1_001,
            &mut sink,
        )
        .expect("non-empty retry input is rejected as a receipt");
    let rejected = sink
        .as_slice()
        .iter()
        .filter_map(emitted_event)
        .find(|event| event["event"] == "im:interaction:action-receipt")
        .expect("invalid retry invocation emits a receipt");
    assert_eq!(rejected["data"]["status"], "rejected");
    assert_eq!(rejected["data"]["validationErrors"][0]["field"], "input");
    assert!(sink
        .as_slice()
        .iter()
        .all(|effect| !matches!(effect, Effect::Http { .. } | Effect::Persist { .. })));
    assert_eq!(
        module
            .state
            .pending_sends
            .get(&temporary_id)
            .and_then(|pending| pending.timeline_readback.causation_id.as_deref()),
        None,
        "prepare rejection must not mutate pending media"
    );

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
        .expect("accepted retry invokes the canonical local command");
    let accepted = sink
        .as_slice()
        .iter()
        .filter_map(emitted_event)
        .find(|event| event["event"] == "im:interaction:action-receipt")
        .expect("accepted retry publishes a receipt");
    assert_eq!(accepted["data"]["status"], "accepted");
    assert!(sink.as_slice().iter().all(|effect| {
        emitted_event(effect).is_none_or(|event| event["event"] != "im:interaction:frame")
    }));
    assert_eq!(
        module
            .state
            .pending_sends
            .get(&temporary_id)
            .and_then(|pending| pending.timeline_readback.causation_id.as_deref()),
        None,
        "retry reset PersistOk 前不能把内存事实提前推进到新 attempt"
    );
    let retry_persist_corr = sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::PersistAtomic { corr, ops }
                if ops.iter().any(|op| {
                    matches!(
                        op,
                        helix_core::effect::StorageOp::BatchUpsert(spec)
                            if spec.table == "message"
                                && spec.rows.iter().flatten().any(|(column, value)| {
                                    column == "send_status"
                                        && matches!(
                                            value,
                                            helix_core::effect::SqlValue::Text(status)
                                                if status == "sending"
                                        )
                                })
                    )
                }) =>
            {
                Some(*corr)
            }
            _ => None,
        })
        .expect("canonical im_retry_send atomically persists the sending state");
    let reset = module
        .state
        .pending_media_retry_resets
        .get(&retry_persist_corr)
        .expect("retry context must wait behind its atomic persist");
    assert_eq!(reset.channel_id, channel_id);
    assert_eq!(
        reset.timeline_readback.window_token.as_deref(),
        Some("retry-action-window")
    );
    assert_eq!(
        reset.timeline_readback.causation_id.as_deref(),
        Some("retry-action-request")
    );
    assert!(
        sink.as_slice()
            .iter()
            .all(|effect| !matches!(effect, Effect::Http { .. })),
        "PersistOk 前不得启动 Java prepare"
    );

    sink.clear();
    module
        .handle(
            &helix_core::Tick::PortReply {
                corr: retry_persist_corr,
                outcome: helix_core::tick::PortOutcome::Ok(helix_core::tick::ReplyBytes::default()),
            },
            1_003,
            &mut sink,
        )
        .expect("retry sending PersistOk schedules causal timeline readback");
    assert_eq!(
        module
            .state
            .pending_sends
            .get(&temporary_id)
            .and_then(|pending| pending.timeline_readback.causation_id.as_deref()),
        Some("retry-action-request")
    );
    assert_eq!(
        module
            .state
            .pending_sends
            .get(&temporary_id)
            .and_then(|pending| pending.timeline_readback.window_token.as_deref()),
        Some("retry-action-window")
    );
    assert!(sink.as_slice().iter().any(|effect| matches!(
        effect,
        Effect::Http { req, .. } if req.url.ends_with("/oss/getPresignedUrl")
    )));
    let scan_corr = sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Persist { corr, ops }
                if ops.iter().any(|op| {
                    matches!(
                        op,
                        helix_core::effect::StorageOp::Scan(spec) if spec.table == "message"
                    )
                }) =>
            {
                Some(*corr)
            }
            _ => None,
        })
        .expect("retry state re-enters local-first timeline projection");

    sink.clear();
    let sending_rows = serde_json::json!([{
        "temporary_id": temporary_id.0,
        "channel_id": channel_id.as_str(),
        "user_id": "viewer-1",
        "create_at": 10,
        "message": "",
        "type": "FILE",
        "props": "{\"file\":{\"name\":\"report.pdf\",\"upload\":{\"status\":\"preparing\"}}}",
        "send_status": "sending",
        "upload_progress_percent": 0
    }]);
    module
        .handle(
            &helix_core::Tick::PortReply {
                corr: scan_corr,
                outcome: helix_core::tick::PortOutcome::Ok(helix_core::tick::ReplyBytes(
                    bytes::Bytes::from(serde_json::to_vec(&sending_rows).unwrap()),
                )),
            },
            1_004,
            &mut sink,
        )
        .expect("causal sending readback emits the retry terminal frame");
    let causal_frame = sink
        .as_slice()
        .iter()
        .filter_map(emitted_event)
        .find(|event| event["event"] == "im:interaction:frame")
        .expect("retry PersistOk readback emits a typed frame");
    let causal_frame = crate::interaction_frame::InteractionFrameV2::decode(
        &serde_json::to_vec(&causal_frame["data"]).unwrap(),
    )
    .unwrap();
    assert!(crate::projector::action::matching_frame_confirmation(
        &causal_frame,
        "retry-action-request"
    )
    .unwrap()
    .is_some());
    assert_eq!(
        causal_frame.mutations[0].mutation.scope, mutation.mutation.scope,
        "retry terminal Frame must return to the scope that granted the actionRef"
    );
    let crate::interaction_frame::TypedSurfaceMutation::TimelinePatch { patch } =
        &causal_frame.mutations[0].mutation.mutation
    else {
        panic!("retry sending readback must patch the attached timeline")
    };
    let retry = patch.view.items[0]
        .actions
        .iter()
        .find(|action| action.kind == "retry")
        .expect("sending view keeps the typed retry binding disabled");
    assert!(!retry.enabled);
    assert!(
        !module
            .state
            .action_registry
            .complete_after_terminal_frame("retry-action-request"),
        "the causal frame must already have released the accepted action ledger"
    );
}

fn start_durable_retry_action(
    module: &mut ImModule,
    request_id: &str,
    out: &mut EffectSink,
) -> helix_core::Correlation {
    use helix_core::Module;

    let rows_desc = vec![serde_json::json!({
        "temporary_id": "temporary-durable-retry",
        "channel_id": channel_id().as_str(),
        "user_id": "viewer-1",
        "create_at": 10,
        "message": "retry me",
        "type": "TEXT",
        "props": {},
        "send_status": "unsend",
        "actionCapabilities": {"canRetry": true}
    })];
    let mut query = request(20);
    query.window_token = "durable-retry-window".to_string();
    let Effect::Emit { event } = module
        .emit_timeline_snapshot_with_causation(&query, &rows_desc, 1_000, None, None)
        .expect("retryable durable row grants an action")
    else {
        panic!("retryable row must emit a timeline frame")
    };
    let event: serde_json::Value = serde_json::from_slice(event.0.as_ref()).unwrap();
    let frame = crate::interaction_frame::InteractionFrameV2::decode(
        &serde_json::to_vec(&event["data"]).unwrap(),
    )
    .unwrap();
    let mutation = &frame.mutations[0];
    let crate::interaction_frame::TypedSurfaceMutation::TimelineSnapshot { view } =
        &mutation.mutation.mutation
    else {
        panic!("initial durable retry frame must be a snapshot")
    };
    let binding = view.items[0]
        .actions
        .iter()
        .find(|action| action.kind == "retry")
        .expect("failed durable row exposes retry")
        .clone();
    let envelope = crate::interaction_frame::ActionInvocationEnvelope {
        schema_version: crate::interaction_frame::ACTION_INVOCATION_ENVELOPE_SCHEMA_VERSION,
        request_id: request_id.to_string(),
        actor_id: "viewer-1".to_string(),
        surface: mutation.mutation.surface,
        scope: mutation.mutation.scope.clone(),
        surface_epoch: binding.surface_epoch,
        observed_revision: mutation.mutation.revision,
        invocation: crate::interaction_frame::ActionInvocation {
            action_ref: binding.action_ref,
            input: std::collections::BTreeMap::new(),
        },
    };
    module
        .handle(
            &helix_core::Tick::Command(
                crate::client_api::build_action_invocation_command(
                    &serde_json::to_vec(&envelope).unwrap(),
                )
                .unwrap(),
            ),
            1_001,
            out,
        )
        .expect("retry action is accepted");
    out.as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Persist { corr, ops }
                if ops.iter().any(|op| {
                    matches!(
                        op,
                        helix_core::effect::StorageOp::Get(spec)
                            if spec.table == "message" && spec.key_col == "temporary_id"
                    )
                }) =>
            {
                Some(*corr)
            }
            _ => None,
        })
        .expect("durable retry starts with a message lookup")
}

#[test]
fn durable_retry_lookup_failure_survives_query_supersession_and_releases_action() {
    use helix_core::Module;

    let mut config = crate::module::ImConfig::default();
    config.auth_user_id = "viewer-1".to_string();
    config.api_base_url = "http://go.test".to_string();
    let mut module = ImModule::new(config);
    let mut sink = EffectSink::new();
    let request_id = "durable-retry-storage-error";
    let lookup_corr = start_durable_retry_action(&mut module, request_id, &mut sink);

    sink.clear();
    module
        .handle(
            &helix_core::Tick::PortReply {
                corr: lookup_corr,
                outcome: helix_core::tick::PortOutcome::Err(helix_core::tick::PortError::Storage(
                    1,
                )),
            },
            1_002,
            &mut sink,
        )
        .expect("lookup failure schedules a causal local readback");
    let causal_scan = sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Persist { corr, ops }
                if ops.iter().any(|op| {
                    matches!(
                        op,
                        helix_core::effect::StorageOp::Scan(spec) if spec.table == "message"
                    )
                }) =>
            {
                Some(*corr)
            }
            _ => None,
        })
        .expect("lookup failure must schedule the original timeline readback");

    let payload = serde_json::to_vec(&serde_json::json!({
        "channel_id": channel_id().as_str(),
        "window_token": "durable-retry-window",
        "limit": 20
    }))
    .unwrap();
    module
        .dispatch_message_query(&payload, &mut EffectSink::new())
        .expect("a newer renderer query supersedes the ordinary generation");

    sink.clear();
    module
        .handle(
            &helix_core::Tick::PortReply {
                corr: causal_scan,
                outcome: helix_core::tick::PortOutcome::Err(helix_core::tick::PortError::Storage(
                    2,
                )),
            },
            1_003,
            &mut sink,
        )
        .expect("causal failure still emits a terminal frame");
    let frame = sink
        .as_slice()
        .iter()
        .filter_map(emitted_event)
        .find(|event| event["event"] == "im:interaction:frame")
        .expect("causal storage failure emits a frame");
    let frame = crate::interaction_frame::InteractionFrameV2::decode(
        &serde_json::to_vec(&frame["data"]).unwrap(),
    )
    .unwrap();
    assert!(
        crate::projector::action::matching_frame_confirmation(&frame, request_id)
            .unwrap()
            .is_some()
    );
    assert!(
        !module
            .state
            .action_registry
            .complete_after_terminal_frame(request_id),
        "terminal failed frame already releases the accepted action ledger"
    );
}

#[test]
fn durable_retry_not_found_falls_back_to_causal_timeline_readback() {
    use helix_core::Module;

    let mut config = crate::module::ImConfig::default();
    config.auth_user_id = "viewer-1".to_string();
    config.api_base_url = "http://go.test".to_string();
    let mut module = ImModule::new(config);
    let mut sink = EffectSink::new();
    let temporary_lookup =
        start_durable_retry_action(&mut module, "durable-retry-not-found", &mut sink);

    sink.clear();
    module
        .handle(
            &helix_core::Tick::PortReply {
                corr: temporary_lookup,
                outcome: helix_core::tick::PortOutcome::Ok(helix_core::tick::ReplyBytes(
                    bytes::Bytes::from_static(b"[]"),
                )),
            },
            1_002,
            &mut sink,
        )
        .unwrap();
    let id_lookup = sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Persist { corr, ops }
                if ops.iter().any(|op| {
                    matches!(
                        op,
                        helix_core::effect::StorageOp::Get(spec)
                            if spec.table == "message" && spec.key_col == "id"
                    )
                }) =>
            {
                Some(*corr)
            }
            _ => None,
        })
        .expect("temporary-id miss falls back to server id");

    sink.clear();
    module
        .handle(
            &helix_core::Tick::PortReply {
                corr: id_lookup,
                outcome: helix_core::tick::PortOutcome::Ok(helix_core::tick::ReplyBytes(
                    bytes::Bytes::from_static(b"[]"),
                )),
            },
            1_003,
            &mut sink,
        )
        .expect("final miss schedules a causal terminal readback");
    assert!(sink.as_slice().iter().any(|effect| matches!(
        effect,
        Effect::Persist { ops, .. }
            if ops.iter().any(|op| matches!(
                op,
                helix_core::effect::StorageOp::Scan(spec) if spec.table == "message"
            ))
    )));
}

#[test]
fn view_receipt_action_uses_authority_and_only_matching_frame_confirms_completion() {
    use helix_core::Module;

    let mut config = crate::module::ImConfig::default();
    config.auth_user_id = "viewer-1".to_string();
    config.api_base_url = "http://go.test".to_string();
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
        .emit_timeline_snapshot_with_causation(&query, &rows_desc, 1_000, None, None)
        .expect("author timeline emits an action-bound Frame");
    let Effect::Emit { event } = timeline_effect else {
        panic!("timeline must emit a frame")
    };
    let timeline_event: serde_json::Value = serde_json::from_slice(event.0.as_ref()).unwrap();
    let timeline_frame = crate::interaction_frame::InteractionFrameV2::decode(
        &serde_json::to_vec(&timeline_event["data"]).unwrap(),
    )
    .unwrap();
    let mutation = &timeline_frame.mutations[0].mutation;
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
    assert_eq!(
        binding.surface_epoch,
        module.action_surface_epoch().unwrap(),
        "renderer must obtain the action epoch from the binding rather than infer it"
    );

    let envelope = crate::interaction_frame::ActionInvocationEnvelope {
        schema_version: crate::interaction_frame::ACTION_INVOCATION_ENVELOPE_SCHEMA_VERSION,
        request_id: "receipt-request-1".to_string(),
        actor_id: "viewer-1".to_string(),
        surface: mutation.surface,
        scope: mutation.scope.clone(),
        surface_epoch: binding.surface_epoch,
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

    // 回归锚: T080 —— 同一仍 live 的 actionRef 不得并发第二个 requestId。
    sink.clear();
    let mut second_request = envelope.clone();
    second_request.request_id = "receipt-request-second".to_string();
    module
        .handle(
            &helix_core::Tick::Command(
                crate::client_api::build_action_invocation_command(
                    &serde_json::to_vec(&second_request).unwrap(),
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
    let second_rejection = sink
        .as_slice()
        .iter()
        .filter_map(emitted_event)
        .find(|event| event["event"] == "im:interaction:action-receipt")
        .expect("second request on a live actionRef is explicitly rejected");
    assert_eq!(
        second_rejection["data"]["validationErrors"][0]["field"],
        "actionRef"
    );

    sink.clear();
    module
        .handle(
            &helix_core::Tick::Command(
                crate::client_api::build_action_invocation_command(
                    &serde_json::to_vec(&envelope).unwrap(),
                )
                .unwrap(),
            ),
            1_004,
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

    // 回归锚: T079 —— 新 timeline revision 撤旧 ref 后，旧 corr 尚未终态时 requestId 仍不可复用。
    let refreshed_timeline = module
        .emit_timeline_snapshot_with_causation(&query, &rows_desc, 1_005, None, None)
        .expect("new timeline revision revokes the old action capability");
    let Effect::Emit { event } = refreshed_timeline else {
        panic!("refreshed timeline must emit a frame")
    };
    let refreshed_event: serde_json::Value = serde_json::from_slice(event.0.as_ref()).unwrap();
    let refreshed_frame = crate::interaction_frame::InteractionFrameV2::decode(
        &serde_json::to_vec(&refreshed_event["data"]).unwrap(),
    )
    .unwrap();
    let refreshed_mutation = &refreshed_frame.mutations[0].mutation;
    let crate::interaction_frame::TypedSurfaceMutation::TimelinePatch { patch } =
        &refreshed_mutation.mutation
    else {
        panic!("attached timeline refresh must be a replace-view patch")
    };
    let view = &patch.view;
    let refreshed_binding = view.items[0]
        .actions
        .iter()
        .find(|action| action.kind == "viewReceipt")
        .expect("refreshed author timeline has a new receipt capability");
    assert_ne!(binding.action_ref, refreshed_binding.action_ref);
    let mut reused_request = envelope.clone();
    reused_request.observed_revision = refreshed_mutation.revision;
    reused_request.invocation.action_ref = refreshed_binding.action_ref.clone();
    sink.clear();
    module
        .handle(
            &helix_core::Tick::Command(
                crate::client_api::build_action_invocation_command(
                    &serde_json::to_vec(&reused_request).unwrap(),
                )
                .unwrap(),
            ),
            1_006,
            &mut sink,
        )
        .unwrap();
    assert!(sink
        .as_slice()
        .iter()
        .all(|effect| !matches!(effect, Effect::Http { .. })));
    let stale_request_rejection = sink
        .as_slice()
        .iter()
        .filter_map(emitted_event)
        .find(|event| event["event"] == "im:interaction:action-receipt")
        .expect("new action must reject the old requestId while its corr is live");
    assert_eq!(
        stale_request_rejection["data"]["validationErrors"][0]["field"],
        "requestId"
    );

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
            1_007,
            &mut sink,
        )
        .unwrap();
    let authority_frame = sink
        .as_slice()
        .iter()
        .filter_map(emitted_event)
        .find(|event| event["event"] == "im:interaction:frame")
        .expect("canonical HTTP reply produces a typed Frame");
    let authority_frame = crate::interaction_frame::InteractionFrameV2::decode(
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

    // The old corr has already been removed before the terminal Frame released requestId. Reuse is now safe,
    // and a duplicated old HTTP reply no longer has a corr_map route that could confirm the fresh request.
    sink.clear();
    module
        .handle(
            &helix_core::Tick::Command(
                crate::client_api::build_action_invocation_command(
                    &serde_json::to_vec(&reused_request).unwrap(),
                )
                .unwrap(),
            ),
            1_008,
            &mut sink,
        )
        .unwrap();
    let fresh_corr = sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Http { corr, .. } => Some(*corr),
            _ => None,
        })
        .expect("terminal action frame releases the requestId for a fresh action");
    assert_ne!(corr, fresh_corr);

    sink.clear();
    module
        .handle(
            &helix_core::Tick::PortReply {
                corr,
                outcome: helix_core::tick::PortOutcome::Ok(http_envelope(serde_json::json!({
                    "status": "SUCCESS",
                    "data": []
                }))),
            },
            1_009,
            &mut sink,
        )
        .unwrap();
    assert!(sink.as_slice().is_empty());
}

#[test]
fn identity_reset_drops_old_view_receipt_corr_before_a_new_action_can_start() {
    use helix_core::Module;

    let mut config = crate::module::ImConfig::default();
    config.auth_user_id = "viewer-before".to_string();
    config.company_id = "team-before".to_string();
    config.api_base_url = "http://go.test".to_string();
    let mut module = ImModule::new(config);
    let mut query = request(20);
    query.window_token = "identity-reset-window".to_string();
    let before_rows = vec![serde_json::json!({
        "id": "message-before",
        "temporaryId": "temporary-before",
        "channelId": channel_id().as_str(),
        "userId": "viewer-before",
        "createAt": 10,
        "message": "before",
        "type": "TEXT",
        "props": {},
    })];
    let Effect::Emit { event } = module
        .emit_timeline_snapshot_with_causation(&query, &before_rows, 1_000, None, None)
        .expect("old identity renders an action-bound timeline")
    else {
        panic!("timeline must emit a Frame")
    };
    let old_frame: serde_json::Value = serde_json::from_slice(event.0.as_ref()).unwrap();
    let old_frame = crate::interaction_frame::InteractionFrameV2::decode(
        &serde_json::to_vec(&old_frame["data"]).unwrap(),
    )
    .unwrap();
    let old_mutation = &old_frame.mutations[0].mutation;
    let crate::interaction_frame::TypedSurfaceMutation::TimelineSnapshot { view } =
        &old_mutation.mutation
    else {
        panic!("old timeline must attach a snapshot")
    };
    let old_binding = view.items[0]
        .actions
        .iter()
        .find(|action| action.kind == "viewReceipt")
        .expect("old viewer owns the message");
    let old_envelope = crate::interaction_frame::ActionInvocationEnvelope {
        schema_version: crate::interaction_frame::ACTION_INVOCATION_ENVELOPE_SCHEMA_VERSION,
        request_id: "old-identity-request".to_string(),
        actor_id: "viewer-before".to_string(),
        surface: old_mutation.surface,
        scope: old_mutation.scope.clone(),
        surface_epoch: old_binding.surface_epoch,
        observed_revision: old_mutation.revision,
        invocation: crate::interaction_frame::ActionInvocation {
            action_ref: old_binding.action_ref.clone(),
            input: std::collections::BTreeMap::new(),
        },
    };
    let mut sink = EffectSink::new();
    module
        .handle(
            &helix_core::Tick::Command(
                crate::client_api::build_action_invocation_command(
                    &serde_json::to_vec(&old_envelope).unwrap(),
                )
                .unwrap(),
            ),
            1_001,
            &mut sink,
        )
        .unwrap();
    let old_corr = sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Http { corr, .. } => Some(*corr),
            _ => None,
        })
        .expect("old action starts its canonical HTTP request");

    sink.clear();
    module
        .handle(
            &helix_core::Tick::Command(helix_core::tick::AppCommand::new(
                crate::module::RUNTIME_IDENTITY_COMMAND,
                serde_json::to_vec(&serde_json::json!({
                    "auth_user_id": "viewer-after",
                    "company_id": "team-after",
                }))
                .unwrap(),
            )),
            1_002,
            &mut sink,
        )
        .unwrap();
    assert!(sink.as_slice().is_empty());

    let after_rows = vec![serde_json::json!({
        "id": "message-after",
        "temporaryId": "temporary-after",
        "channelId": channel_id().as_str(),
        "userId": "viewer-after",
        "createAt": 11,
        "message": "after",
        "type": "TEXT",
        "props": {},
    })];
    let Effect::Emit { event } = module
        .emit_timeline_snapshot_with_causation(&query, &after_rows, 1_003, None, None)
        .expect("new identity renders its own action-bound timeline")
    else {
        panic!("timeline must emit a Frame")
    };
    let after_frame: serde_json::Value = serde_json::from_slice(event.0.as_ref()).unwrap();
    let after_frame = crate::interaction_frame::InteractionFrameV2::decode(
        &serde_json::to_vec(&after_frame["data"]).unwrap(),
    )
    .unwrap();
    let after_mutation = &after_frame.mutations[0].mutation;
    let crate::interaction_frame::TypedSurfaceMutation::TimelineSnapshot { view } =
        &after_mutation.mutation
    else {
        panic!("identity reset detaches the old timeline before new attach")
    };
    let after_binding = view.items[0]
        .actions
        .iter()
        .find(|action| action.kind == "viewReceipt")
        .expect("new viewer owns the new message");
    let after_envelope = crate::interaction_frame::ActionInvocationEnvelope {
        schema_version: crate::interaction_frame::ACTION_INVOCATION_ENVELOPE_SCHEMA_VERSION,
        request_id: "new-identity-request".to_string(),
        actor_id: "viewer-after".to_string(),
        surface: after_mutation.surface,
        scope: after_mutation.scope.clone(),
        surface_epoch: after_binding.surface_epoch,
        observed_revision: after_mutation.revision,
        invocation: crate::interaction_frame::ActionInvocation {
            action_ref: after_binding.action_ref.clone(),
            input: std::collections::BTreeMap::new(),
        },
    };
    sink.clear();
    module
        .handle(
            &helix_core::Tick::Command(
                crate::client_api::build_action_invocation_command(
                    &serde_json::to_vec(&after_envelope).unwrap(),
                )
                .unwrap(),
            ),
            1_004,
            &mut sink,
        )
        .unwrap();
    assert!(sink
        .as_slice()
        .iter()
        .any(|effect| matches!(effect, Effect::Http { .. })));

    // The old reply has no corr route after reset. It cannot mutate or complete the new identity action.
    sink.clear();
    module
        .handle(
            &helix_core::Tick::PortReply {
                corr: old_corr,
                outcome: helix_core::tick::PortOutcome::Ok(http_envelope(serde_json::json!({
                    "status": "SUCCESS",
                    "data": [{
                        "postId": "message-before",
                        "authorUserId": "viewer-before",
                        "snapshotId": "old-snapshot",
                        "snapshotVersion": 1,
                        "memberIds": ["viewer-before"],
                        "readBits": "1"
                    }]
                }))),
            },
            1_005,
            &mut sink,
        )
        .unwrap();
    assert!(sink.as_slice().is_empty());

    sink.clear();
    module
        .handle(
            &helix_core::Tick::Command(
                crate::client_api::build_action_invocation_command(
                    &serde_json::to_vec(&after_envelope).unwrap(),
                )
                .unwrap(),
            ),
            1_006,
            &mut sink,
        )
        .unwrap();
    assert!(sink
        .as_slice()
        .iter()
        .all(|effect| !matches!(effect, Effect::Http { .. })));
    let rejection = sink
        .as_slice()
        .iter()
        .filter_map(emitted_event)
        .find(|event| event["event"] == "im:interaction:action-receipt")
        .expect("the new action remains in flight after the old late reply");
    assert_eq!(
        rejection["data"]["validationErrors"][0]["field"],
        "requestId"
    );
}
