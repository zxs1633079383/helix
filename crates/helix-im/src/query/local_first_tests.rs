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
        .emit_timeline_snapshot_with_causation(&request, &rows_desc, 1_000, None)
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
        .emit_timeline_snapshot_with_causation(&request(50), &initial_rows, 1_000, None)
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
    let frame = crate::interaction_frame::InteractionFrame::decode(
        &serde_json::to_vec(&frame_event["data"]).unwrap(),
    )
    .expect("the refresh output satisfies the public InteractionFrame contract");
    let mutation = frame
        .mutations
        .iter()
        .find(|mutation| mutation.surface == crate::interaction_frame::SurfaceId::MessageTimeline)
        .expect("the Frame includes the timeline surface");
    assert_eq!(mutation.causation_id.as_deref(), Some("send-request-1"));
    assert!(matches!(
        mutation.mutation,
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
        .emit_timeline_snapshot_with_causation(&request(50), &initial_rows, 2_000, None)
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
    let frame = crate::interaction_frame::InteractionFrame::decode(
        &serde_json::to_vec(&frame_event["data"]).unwrap(),
    )
    .expect("the terminal projection satisfies the Frame contract");
    let mutation = frame
        .mutations
        .iter()
        .find(|mutation| mutation.surface == crate::interaction_frame::SurfaceId::MessageTimeline)
        .expect("the terminal Frame patches the timeline");
    assert_eq!(mutation.causation_id.as_deref(), Some("send-request-2"));
    assert!(matches!(
        mutation.mutation,
        crate::interaction_frame::TypedSurfaceMutation::TimelinePatch { .. }
    ));
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
        .emit_timeline_snapshot_with_causation(&query, &rows_desc, 1_000, None)
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
        .emit_timeline_snapshot_with_causation(&query, &rows_desc, 1_005, None)
        .expect("new timeline revision revokes the old action capability");
    let Effect::Emit { event } = refreshed_timeline else {
        panic!("refreshed timeline must emit a frame")
    };
    let refreshed_event: serde_json::Value = serde_json::from_slice(event.0.as_ref()).unwrap();
    let refreshed_frame = crate::interaction_frame::InteractionFrame::decode(
        &serde_json::to_vec(&refreshed_event["data"]).unwrap(),
    )
    .unwrap();
    let refreshed_mutation = &refreshed_frame.mutations[0];
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
        .emit_timeline_snapshot_with_causation(&query, &before_rows, 1_000, None)
        .expect("old identity renders an action-bound timeline")
    else {
        panic!("timeline must emit a Frame")
    };
    let old_frame: serde_json::Value = serde_json::from_slice(event.0.as_ref()).unwrap();
    let old_frame = crate::interaction_frame::InteractionFrame::decode(
        &serde_json::to_vec(&old_frame["data"]).unwrap(),
    )
    .unwrap();
    let old_mutation = &old_frame.mutations[0];
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
        .emit_timeline_snapshot_with_causation(&query, &after_rows, 1_003, None)
        .expect("new identity renders its own action-bound timeline")
    else {
        panic!("timeline must emit a Frame")
    };
    let after_frame: serde_json::Value = serde_json::from_slice(event.0.as_ref()).unwrap();
    let after_frame = crate::interaction_frame::InteractionFrame::decode(
        &serde_json::to_vec(&after_frame["data"]).unwrap(),
    )
    .unwrap();
    let after_mutation = &after_frame.mutations[0];
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
