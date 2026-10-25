use super::*;
use helix_core::effect::{
    BatchDeleteSpec, ScopedScanSpec, SqlValue, StorageOp, UpsertGuard, UpsertSpec,
};
use serde_json::json;

pub(super) fn key(channel: ChannelId, seq: i64) -> String {
    format!("{}:{seq}", channel.as_str())
}

pub(super) fn slot_scan(p: &Pending) -> StorageOp {
    StorageOp::ScopedScan(ScopedScanSpec {
        table: "message_sequence_slot",
        scope_col: "slot_key",
        scope_values: (p.lo..=p.hi)
            .map(|seq| SqlValue::Text(key(p.request.channel, seq)))
            .collect(),
        filter: Some((
            "channel_id",
            SqlValue::Text(p.request.channel.as_str().to_owned()),
        )),
        limit: RANGE_SIZE as usize,
    })
}

pub(super) fn body_scan(p: &Pending) -> StorageOp {
    StorageOp::ScopedScan(ScopedScanSpec {
        table: "message",
        scope_col: "temporary_id",
        scope_values: p
            .previous_keys
            .values()
            .cloned()
            .map(SqlValue::Text)
            .collect(),
        filter: Some((
            "channel_id",
            SqlValue::Text(p.request.channel.as_str().to_owned()),
        )),
        limit: RANGE_SIZE as usize,
    })
}

pub(super) fn rows(raw: &[u8]) -> Result<Vec<Value>, ImError> {
    let rows: Vec<Value> = serde_json::from_slice(raw).map_err(|_| error("INVALID_LOCAL_ROWS"))?;
    if rows.len() > RANGE_SIZE as usize || rows.iter().any(|v| !v.is_object()) {
        return Err(error("INVALID_LOCAL_ROWS"));
    }
    Ok(rows)
}

pub(super) fn ingest_slots(p: &mut Pending, raw: &[u8]) -> Result<(), ImError> {
    let meta = p.meta.as_ref().ok_or_else(|| error("MISSING_METADATA"))?;
    p.slots.clear();
    p.previous_keys.clear();
    p.previous_revisions.clear();
    for row in rows(raw)? {
        let seq = row["seq"]
            .as_i64()
            .ok_or_else(|| error("INVALID_LOCAL_SEQUENCE"))?;
        if seq < p.lo
            || seq > p.hi
            || row["channel_id"] != p.request.channel.as_str()
            || row["slot_key"] != key(p.request.channel, seq)
        {
            return Err(error("INVALID_LOCAL_SCOPE"));
        }
        let visible = row["is_visible"]
            .as_i64()
            .ok_or_else(|| error("INVALID_LOCAL_VISIBILITY"))?;
        let key = match visible {
            0 if row["temporary_id"].is_null() => None,
            1 => Some(
                row["temporary_id"]
                    .as_str()
                    .filter(|v| !v.is_empty())
                    .ok_or_else(|| error("INVALID_LOCAL_KEY"))?
                    .to_owned(),
            ),
            _ => return Err(error("INVALID_LOCAL_VISIBILITY")),
        };
        if let Some(key) = &key {
            p.previous_keys.insert(seq, key.clone());
        }
        p.previous_revisions.insert(
            seq,
            row["revision"]
                .as_i64()
                .ok_or_else(|| error("INVALID_LOCAL_REVISION"))?,
        );
        if row["membership_epoch"].as_i64() != Some(meta.epoch)
            || row["revision"].as_i64() != Some(meta.revision)
        {
            continue;
        }
        p.slots.insert(
            seq,
            Slot {
                seq,
                key,
                event_seq: row["body_event_seq"].as_i64().unwrap_or(0),
            },
        );
    }
    Ok(())
}

pub(super) fn ingest_bodies(p: &mut Pending, raw: &[u8]) -> Result<(), ImError> {
    p.bodies.clear();
    for mut row in rows(raw)? {
        if row["channel_id"] != p.request.channel.as_str() {
            return Err(error("INVALID_LOCAL_SCOPE"));
        }
        let key = row["temporary_id"]
            .as_str()
            .filter(|v| !v.is_empty())
            .ok_or_else(|| error("INVALID_LOCAL_KEY"))?
            .to_owned();
        clear_revoked_body(&mut row);
        p.bodies.insert(key, row);
    }
    Ok(())
}

/// A slot without its visible body is an unknown gap, never proof of exhaustion.
pub(super) fn complete(p: &Pending, seq: i64) -> bool {
    let Some(slot) = p.slots.get(&seq) else {
        return false;
    };
    match &slot.key {
        None => true,
        Some(key) => p
            .bodies
            .get(key)
            .is_some_and(|row| row["event_seq"].as_i64().unwrap_or(0) >= slot.event_seq),
    }
}

pub(super) fn missing(p: &Pending) -> Option<(i64, i64)> {
    let first = (p.lo..=p.hi).find(|seq| !complete(p, *seq))?;
    let last = (first..=p.hi)
        .take_while(|seq| !complete(p, *seq))
        .last()
        .unwrap_or(first);
    Some((first, last))
}

pub(super) fn metadata(data: &Value) -> Result<Metadata, ImError> {
    let first = decimal(&data["firstSeq"])?;
    let head = decimal(&data["snapshotHeadSeq"])?;
    if first > head || (first == 0) != (head == 0) || head > decimal(&data["headSeq"])? {
        return Err(error("INVALID_METADATA"));
    }
    let anchor = data.get("anchorSeq").map(decimal).transpose()?;
    if anchor.is_some_and(|seq| seq < first || seq > head || seq == 0) {
        return Err(error("INVALID_ANCHOR"));
    }
    Ok(Metadata {
        first,
        head,
        epoch: decimal(&data["membershipEpoch"])?,
        revision: decimal(&data["revision"])?,
        anchor,
    })
}

pub(super) fn range_ops(
    p: &Pending,
    data: &Value,
    lo: i64,
    hi: i64,
    viewer: &str,
) -> Result<Vec<StorageOp>, ImError> {
    let meta = p.meta.as_ref().ok_or_else(|| error("MISSING_METADATA"))?;
    let response = metadata(data)?;
    if response.head != meta.head
        || response.epoch != meta.epoch
        || response.revision != meta.revision
        || decimal(&data["fromSeq"])? != lo
        || decimal(&data["toSeq"])? != hi
    {
        return Err(error("SNAPSHOT_CHANGED"));
    }
    let slots = data["slots"]
        .as_array()
        .filter(|v| v.len() == (hi - lo + 1) as usize)
        .ok_or_else(|| error("INCOMPLETE_RANGE"))?;
    let mut post_rows = Vec::new();
    let mut revocations = BTreeMap::new();
    let mut slot_rows = Vec::new();
    let mut purge_ops = Vec::new();
    for (offset, slot) in slots.iter().enumerate() {
        let seq = decimal(&slot["seq"])?;
        if seq != lo + offset as i64 {
            return Err(error("INCOMPLETE_RANGE"));
        }
        let visible = slot["isVisible"]
            .as_bool()
            .ok_or_else(|| error("INVALID_VISIBILITY"))?;
        let (temporary, event_seq) = if visible {
            let mut post = slot["post"].clone();
            clear_revoked_body(&mut post);
            let event_seq = decimal(&slot["postEventSeq"])?;
            let object = post.as_object_mut().ok_or_else(|| error("MISSING_BODY"))?;
            if field(&Value::Object(object.clone()), "channel_id", "channelId")
                != p.request.channel.as_str()
            {
                return Err(error("INVALID_REMOTE_SCOPE"));
            }
            object.insert("eventSeq".into(), json!(event_seq));
            let fields = crate::ws::parser::extract_post_fields(&post);
            if !crate::channel_write::post_updates_from_fields(p.request.channel, &fields, viewer)
                .visible
            {
                return Err(error("REMOTE_VISIBILITY_CONFLICT"));
            }
            let temporary = if fields.temporary_id.is_empty() {
                fields.id
            } else {
                fields.temporary_id
            };
            if temporary.is_empty() {
                return Err(error("MISSING_BODY_KEY"));
            }
            revocations.insert(temporary.clone(), revoked(&post));
            post_rows.push(post);
            (SqlValue::Text(temporary), event_seq)
        } else {
            if slot.get("post").is_some() || slot.get("postEventSeq").is_some() {
                return Err(error("HIDDEN_BODY"));
            }
            if let Some((key, row)) = p
                .previous_keys
                .get(&seq)
                .and_then(|key| p.bodies.get(key).map(|row| (key, row)))
            {
                // Guard plus delete share one transaction: an intervening live update aborts the purge.
                let mut guard = UpsertSpec::new(
                    "message",
                    vec![vec![
                        ("temporary_id".into(), SqlValue::Text(key.clone())),
                        (
                            "channel_id".into(),
                            SqlValue::Text(p.request.channel.as_str().to_owned()),
                        ),
                        (
                            "event_seq".into(),
                            SqlValue::Integer(row["event_seq"].as_i64().unwrap_or(0)),
                        ),
                    ]],
                    Some("temporary_id"),
                );
                guard.update_guard = Some(UpsertGuard {
                    column: "event_seq",
                    expected: row["event_seq"]
                        .as_i64()
                        .map_or(SqlValue::Null, SqlValue::Integer),
                });
                purge_ops.push(StorageOp::BatchUpsert(guard));
                purge_ops.push(StorageOp::BatchDelete(BatchDeleteSpec {
                    table: "message",
                    scope_col: "channel_id",
                    scope_val: SqlValue::Text(p.request.channel.as_str().to_owned()),
                    key_col: "temporary_id",
                    key_vals: vec![SqlValue::Text(key.clone())],
                }));
            }
            (SqlValue::Null, 0)
        };
        slot_rows.push(vec![
            (
                "slot_key".into(),
                SqlValue::Text(key(p.request.channel, seq)),
            ),
            (
                "channel_id".into(),
                SqlValue::Text(p.request.channel.as_str().to_owned()),
            ),
            ("seq".into(), SqlValue::Integer(seq)),
            ("is_visible".into(), SqlValue::Integer(i64::from(visible))),
            ("temporary_id".into(), temporary),
            ("membership_epoch".into(), SqlValue::Integer(meta.epoch)),
            ("revision".into(), SqlValue::Integer(meta.revision)),
            ("body_event_seq".into(), SqlValue::Integer(event_seq)),
        ]);
    }
    let local = p.bodies.values().cloned().collect::<Vec<_>>();
    let (_, mut ops) = crate::query::local_first::visible_remote_rows_and_cache_ops(
        p.request.channel,
        post_rows,
        &local,
        viewer,
    )?;
    for op in &mut ops {
        if let StorageOp::BatchUpsert(spec) = op {
            // HTTP cannot overwrite a newer message that arrived through the unchanged event path.
            spec.version_column = Some("event_seq");
            for row in &mut spec.rows {
                let withdrawn = row
                    .iter()
                    .find_map(|(column, value)| match (column.as_str(), value) {
                        ("temporary_id", SqlValue::Text(key)) => revocations.get(key),
                        _ => None,
                    })
                    .copied()
                    .ok_or_else(|| error("MISSING_BODY_KEY"))?;
                // The legacy PostFields upsert has no revoke/quick_reply mapping. Keep these
                // sequence-snapshot fields in the same version-guarded write as the cleared body.
                row.push(("revoke".into(), SqlValue::Integer(i64::from(withdrawn))));
                if withdrawn {
                    row.push(("quick_reply".into(), SqlValue::Text("[]".into())));
                }
            }
        }
    }
    ops.extend(purge_ops);
    for (offset, row) in slot_rows.into_iter().enumerate() {
        // CAS fences the whole transaction, including bodies whose event version did not change.
        let mut slot = UpsertSpec::new("message_sequence_slot", vec![row], Some("slot_key"));
        slot.version_column = Some("revision");
        slot.update_guard = Some(UpsertGuard {
            column: "revision",
            expected: SqlValue::Integer(
                p.previous_revisions
                    .get(&(lo + offset as i64))
                    .copied()
                    .unwrap_or(meta.revision),
            ),
        });
        ops.push(StorageOp::BatchUpsert(slot));
    }
    Ok(ops)
}

pub(super) fn cursor(p: &Pending, direction: Direction, next: i64) -> Value {
    let Some(meta) = &p.meta else {
        return Value::Null;
    };
    json!({"version":2,"channelId":p.request.channel.as_str(),"direction":if direction==Direction::Older{"older"}else{"newer"},
        "nextSeq":next.to_string(),"headSeq":meta.head.to_string(),"membershipEpoch":meta.epoch.to_string(),"revision":meta.revision.to_string()})
}

/// Emit only durable rows, in sequence order. Cursors cover scanned invisible slots too.
pub(super) fn result(p: &Pending, viewer: &str) -> Result<Value, ImError> {
    let meta = p.meta.as_ref().ok_or_else(|| error("MISSING_METADATA"))?;
    let mut values = p
        .rows
        .iter()
        .map(|(seq, row)| {
            let mut row = row.clone();
            row["seq"] = json!(seq.to_string());
            row
        })
        .collect::<Vec<_>>();
    if values.len() > p.request.size {
        if p.direction == Direction::Older {
            values.drain(..values.len() - p.request.size);
        } else {
            values.truncate(p.request.size);
        }
    }
    let target_index = values.iter().position(|row| {
        crate::timeline_navigation::row_matches_message_identity(row, &p.request.anchor)
    });
    if p.request.operation == "locate" && target_index.is_none() {
        return Err(error("ANCHOR_UNAVAILABLE"));
    }
    let lo = p.accepted_lo.unwrap_or(if meta.head == 0 {
        0
    } else if p.direction == Direction::Older {
        meta.first
    } else {
        meta.head.saturating_add(1)
    });
    let hi = p.accepted_hi.unwrap_or(if meta.head == 0 {
        0
    } else if p.direction == Direction::Older {
        meta.first.saturating_sub(1)
    } else {
        meta.head
    });
    let has_older = lo > meta.first && meta.first > 0;
    let has_newer = hi < meta.head;
    let sequences = values
        .iter()
        .filter_map(|row| {
            let id = row["id"]
                .as_str()
                .filter(|id| !id.is_empty())
                .or_else(|| field(row, "temporary_id", "temporaryId").as_str())?;
            Some((id.to_owned(), row["seq"].clone()))
        })
        .collect::<BTreeMap<_, _>>();
    let mut shaped =
        crate::render_ready::shape_message_rows_for_viewer(&Value::Array(values), viewer);
    // This read-model field is independent of eventSeq; legacy Post/WS projections stay unchanged.
    for row in shaped.as_array_mut().ok_or_else(|| error("INVALID_ROWS"))? {
        row["postSeq"] = sequences
            .get(row["msgId"].as_str().unwrap_or_default())
            .ok_or_else(|| error("INVALID_ROW_IDENTITY"))?
            .clone();
    }
    let mut result = json!({"reqId":p.request.req_id,"channelId":p.request.channel.as_str(),"operation":p.request.operation,"sequencePagination":true,
        "messages":shaped,"hasOlder":has_older,"hasNewer":has_newer,"pageSize":p.request.size,
        "olderCursor":if has_older{cursor(p,Direction::Older,lo-1)}else{Value::Null},
        "newerCursor":if has_newer{cursor(p,Direction::Newer,hi+1)}else{Value::Null}});
    if p.request.operation == "locate" {
        result["targetPostId"] = json!(p.request.anchor);
        result["targetIndex"] = json!(target_index);
    }
    Ok(result)
}

pub(super) fn revoked(row: &Value) -> bool {
    let v = field(row, "revoke", "isRevoke");
    v.as_bool()
        .unwrap_or_else(|| v.as_i64().is_some_and(|v| v != 0))
}

/// Match Go sequenceVisiblePost's cleared body families in wire and cached-row aliases.
/// Explicit empty fields stop readback's absent-field overlay from restoring withdrawn content.
fn clear_revoked_body(row: &mut Value) {
    if !revoked(row) {
        return;
    }
    let Some(object) = row.as_object_mut() else {
        return;
    };
    for (snake, camel, empty) in [
        ("message", "text", json!("")),
        ("simple_message", "simpleMessage", json!("")),
        ("props", "props", json!({})),
        ("replied_message", "repliedMessage", Value::Null),
        ("reply_messages", "replyMessages", json!({})),
        ("topic", "topic", Value::Null),
        ("quick_reply", "quickReply", json!([])),
        ("expedite_map", "expediteMap", json!({})),
    ] {
        if snake != camel {
            object.insert(camel.into(), empty.clone());
        }
        object.insert(snake.into(), empty);
    }
}
