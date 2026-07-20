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
        .emit_timeline_snapshot(&request, &rows_desc)
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
        .emit_timeline_failed(&request(20))
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
