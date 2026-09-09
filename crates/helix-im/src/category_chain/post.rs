//! Category Post revisions protect the shared message row; other post types retain their write policy.
use crate::error::ImError;
use crate::sync_session::{EventEnvelope, PostFields};
use helix_core::effect::{Row, SqlValue, StorageOp, UpsertSpec};

/// Fixed-width decimal keys preserve the full u64 order in SQLite and the Web store.
pub(crate) fn revision_key(fields: &PostFields) -> Option<String> {
    let props: serde_json::Value = serde_json::from_str(&fields.props).ok()?;
    let revision = props.pointer("/categoryChain/revision")?.as_str()?;
    super::facts::validate_revision(revision).ok()?;
    Some(format!("{:020}", revision.parse::<u64>().ok()?))
}

/// Reject a CATEGORY_CHAIN authority whose canonical decimal revision is absent or malformed.
pub(crate) fn validate_fields(fields: &PostFields) -> Result<(), ImError> {
    if fields.msg_type == "CATEGORY_CHAIN" && revision_key(fields).is_none() {
        return Err(ImError::Parse(
            "CATEGORY_CHAIN post has invalid categoryChain.revision".to_string(),
        ));
    }
    Ok(())
}

/// Only CATEGORY_CHAIN opts into monotonic writes; malformed authority cannot overwrite a durable row.
pub(crate) fn guard_upsert(spec: &mut UpsertSpec, fields: &PostFields) {
    if fields.msg_type != "CATEGORY_CHAIN" {
        return;
    }
    let Some(revision) = revision_key(fields) else {
        spec.rows.clear();
        return;
    };
    spec.version_column = Some("category_revision");
    for row in &mut spec.rows {
        row.push((
            "category_revision".to_owned(),
            SqlValue::Text(revision.clone()),
        ));
    }
}

/// Category edits use the canonical temporary key and retain the existing content-only patch contract.
pub(crate) fn guarded_edit(fields: &PostFields, mut patch: Row) -> StorageOp {
    let mut spec = UpsertSpec::new("message", Vec::new(), Some("temporary_id"));
    if fields.temporary_id.is_empty()
        || crate::state::ChannelId::from_str(&fields.channel_id).is_none()
    {
        return StorageOp::BatchUpsert(spec);
    }
    // Consume the content patch once. Insert defaults never overwrite identity or local read/send state.
    patch.push((
        "temporary_id".to_owned(),
        SqlValue::Text(fields.temporary_id.to_owned()),
    ));
    for (column, value) in [
        ("id", SqlValue::Text(fields.id.to_owned())),
        ("channel_id", SqlValue::Text(fields.channel_id.to_owned())),
        ("user_id", SqlValue::Text(fields.user_id.to_owned())),
        (
            "user_snapshot",
            SqlValue::Text(fields.user_snapshot.to_owned()),
        ),
        ("read_bits", SqlValue::Text(fields.read_bits.to_owned())),
        ("send_status", SqlValue::Text("sent".to_owned())),
        ("create_at", SqlValue::Integer(fields.create_at)),
        ("update_at", SqlValue::Integer(fields.update_at)),
        (
            "viewers",
            SqlValue::Text(serde_json::json!(fields.viewers).to_string()),
        ),
        (
            "simple_message",
            SqlValue::Text(fields.simple_message.to_owned()),
        ),
    ] {
        if !patch.iter().any(|(key, _)| key == column) {
            patch.push((column.to_owned(), value));
            spec.exclude_from_update.push(column);
        }
    }
    spec.rows.push(patch);
    guard_upsert(&mut spec, fields);
    StorageOp::BatchUpsert(spec)
}

/// Category edits retain the updated event contract through the same correlated gate as new posts.
pub(crate) fn queue_edit(
    state: &mut crate::state::ImState,
    corr: helix_core::Correlation,
    event: EventEnvelope,
    out: &mut helix_core::EffectSink,
) {
    if validate_fields(&event.fields).is_err() {
        return;
    }
    let effect = crate::acl::to_effect::emit_post_updated_for_viewer(
        event.channel_id,
        event.seq.0,
        &event.fields.id,
        &event.fields,
        &event.viewer_user_id,
    );
    let helix_core::Effect::Emit { event: bytes } = effect else {
        return;
    };
    let ops = vec![
        crate::channel::edit_content_op(&event.fields.id, &event.fields),
        crate::acl::to_effect::advance_cursor_op(event.channel_id, event.seq),
    ];
    state.corr_map.insert(
        corr,
        crate::state::CorrelationContext::PostUpdateAtomic {
            event: Box::new(event),
            pending_domain_event: bytes.0.to_vec(),
            has_category_posts: true,
        },
    );
    out.push(helix_core::Effect::PersistAtomic { corr, ops });
}
