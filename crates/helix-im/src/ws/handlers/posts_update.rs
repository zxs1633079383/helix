//! `posts_update` action handler（canonical events[] 批量编辑/撤回）。
//!
//! `post_update` 是另一条单条消息协议，本模块只消费 `posts_update.data.events[]`：每个
//! 事件自带权威 `event_seq`，按本地频道 cursor 形成连续前缀后一次 `PersistAtomic` 落库。
//! 根帧 `seq` 与 replyMessages 内部子消息序号均不参与频道游标判断。

use helix_core::{Effect, EffectSink};
use serde_json::Value;

use crate::error::ImError;
use crate::state::{ChannelId, Seq};
use crate::ws::parser::extract_post_fields;

use super::super::{ImWsContext, WsFrame, WsHandlerRegistration, WsMessageHandler};

const POSTS_UPDATE_ACTION: &str = "posts_update";

#[derive(Debug, Clone)]
struct PostsUpdateEvent {
    seq: Seq,
    post: Value,
}

#[derive(Debug)]
struct PostsUpdateBatch {
    channel_id: ChannelId,
    events: Vec<PostsUpdateEvent>,
}

struct PostsUpdateHandler;

impl WsMessageHandler for PostsUpdateHandler {
    fn action(&self) -> &'static str {
        POSTS_UPDATE_ACTION
    }

    /// 将 canonical events[] 的连续前缀与 cursor、业务 patch 一次原子提交。
    fn handle(
        &self,
        ctx: &mut ImWsContext<'_>,
        frame: &WsFrame,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let Ok(data) = frame.data_required() else {
            return Ok(());
        };
        let batch = match parse_posts_update_batch(data) {
            Ok(batch) => batch,
            Err(reason) => {
                log_posts_update_drop(frame, None, None, None, 0, 0, reason);
                return Ok(());
            }
        };
        let Some(initial_cursor) = ctx
            .state
            .channels
            .get(&batch.channel_id)
            .map(|channel| channel.cursor.value())
        else {
            log_posts_update_drop(
                frame,
                Some(batch.channel_id),
                None,
                batch.events.first().map(|event| event.seq),
                0,
                batch.events.len(),
                "unknown_channel",
            );
            return Ok(());
        };

        let mut accepted = Vec::with_capacity(batch.events.len());
        let mut working_cursor = initial_cursor;
        let mut gap_seq = None;
        let mut dropped_count = 0usize;
        for (index, event) in batch.events.iter().enumerate() {
            if event.seq <= initial_cursor {
                dropped_count += 1;
                log_posts_update_drop(
                    frame,
                    Some(batch.channel_id),
                    Some(Seq(initial_cursor.0.saturating_add(1))),
                    Some(event.seq),
                    accepted.len(),
                    dropped_count,
                    "stale_or_duplicate",
                );
                continue;
            }
            let expected = Seq(working_cursor.0.saturating_add(1));
            if event.seq == expected {
                accepted.push(event.clone());
                working_cursor = event.seq;
                continue;
            }
            gap_seq = Some(event.seq);
            dropped_count += batch.events.len().saturating_sub(index);
            log_posts_update_drop(
                frame,
                Some(batch.channel_id),
                Some(expected),
                Some(event.seq),
                accepted.len(),
                dropped_count,
                "gap",
            );
            break;
        }

        if accepted.is_empty() {
            if let Some(gap) = gap_seq {
                arm_posts_update_gap(ctx, batch.channel_id, gap, out);
            }
            return Ok(());
        }

        let mut ops = Vec::with_capacity(accepted.len().saturating_add(1));
        let mut pending_domain_events = Vec::new();
        let mut edit_posts = Vec::new();
        let mut saw_edit = false;
        for event in &accepted {
            let Some(msg_id) = event
                .post
                .get("id")
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty())
            else {
                continue;
            };
            let fields = extract_post_fields(&event.post);
            let is_revoke = event
                .post
                .get("revoke")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if is_revoke {
                ops.push(crate::channel::revoke_authority_op(
                    msg_id,
                    fields,
                    event.seq.0,
                ));
                pending_domain_events.push(
                    crate::event::post::revoke_from_authority(&event.post, event.seq.0)?
                        .into_bytes(),
                );
            } else {
                saw_edit = true;
                ops.push(crate::channel::posts_update_edit_op(
                    msg_id,
                    &fields,
                    event.seq.0,
                ));
                edit_posts.push(event.post.clone());
            }
        }
        if saw_edit {
            if let Effect::Emit { event } = crate::acl::to_effect::emit_post_batch_updated(
                batch.channel_id,
                &Value::Array(edit_posts),
            ) {
                pending_domain_events.push(event.0.to_vec());
            }
        }
        ops.push(crate::acl::to_effect::advance_cursor_op(
            batch.channel_id,
            working_cursor,
        ));
        let corr = ctx.alloc_corr();
        ctx.state.corr_map.insert(
            corr,
            crate::state::CorrelationContext::PostsUpdateAtomic {
                channel_id: batch.channel_id,
                target_seq: working_cursor,
                pending_domain_events,
                refresh_timeline: saw_edit,
            },
        );
        out.push(Effect::PersistAtomic { corr, ops });

        if let Some(gap) = gap_seq {
            arm_posts_update_gap(ctx, batch.channel_id, gap, out);
        }
        Ok(())
    }
}

/// 解析并验证 canonical events[]，不读取根帧 seq 或 nested reply props 序号。
fn parse_posts_update_batch(data: &Value) -> Result<PostsUpdateBatch, &'static str> {
    let Some(events) = data.get("events").and_then(Value::as_array) else {
        return Err("malformed");
    };
    if events.is_empty() {
        return Err("malformed");
    }
    let declared_channel = match data
        .get("channelId")
        .or_else(|| data.get("channel_id"))
        .and_then(Value::as_str)
    {
        Some(value) if !value.is_empty() => {
            Some(ChannelId::from_str(value).ok_or("channel_mismatch")?)
        }
        Some(_) => return Err("malformed"),
        None => None,
    };
    let mut previous = None;
    let mut channel_id = declared_channel;
    let mut parsed = Vec::with_capacity(events.len());
    for event in events {
        let Some(seq) = event
            .get("event_seq")
            .and_then(Value::as_u64)
            .filter(|seq| *seq > 0)
        else {
            return Err("malformed");
        };
        let Some(post) = event.get("post").filter(|post| post.is_object()) else {
            return Err("malformed");
        };
        if post
            .get("id")
            .and_then(Value::as_str)
            .is_none_or(|id| id.is_empty())
        {
            return Err("malformed");
        }
        let seq = Seq(seq);
        if previous.is_some_and(|last| seq <= last) {
            return Err("non_monotonic");
        }
        let post_channel = post
            .get("channelId")
            .or_else(|| post.get("channel_id"))
            .and_then(Value::as_str);
        let post_channel = match post_channel {
            Some(value) if !value.is_empty() => {
                Some(ChannelId::from_str(value).ok_or("channel_mismatch")?)
            }
            Some(_) => return Err("malformed"),
            None => None,
        };
        if declared_channel.is_none() && post_channel.is_none() {
            return Err("malformed");
        }
        if channel_id.is_none() {
            channel_id = post_channel;
        }
        if let (Some(expected), Some(actual)) = (channel_id, post_channel) {
            if expected != actual {
                return Err("channel_mismatch");
            }
        }
        previous = Some(seq);
        parsed.push(PostsUpdateEvent {
            seq,
            post: post.clone(),
        });
    }
    Ok(PostsUpdateBatch {
        channel_id: channel_id.ok_or("malformed")?,
        events: parsed,
    })
}

/// Gap 事件只 arm 既有 gate/backfill，不提升 cursor，也不绕过原子提交。
fn arm_posts_update_gap(
    ctx: &mut ImWsContext<'_>,
    channel_id: ChannelId,
    received_seq: Seq,
    out: &mut EffectSink,
) {
    if let Some(channel) = ctx.state.channels.get_mut(&channel_id) {
        if let Err(error) = channel.ingest_cursor_advance(received_seq, out) {
            tracing::warn!(
                action = POSTS_UPDATE_ACTION,
                channel_id = channel_id.as_str(),
                received_seq = received_seq.0,
                error = ?error,
                "posts_update gap gate arm failed"
            );
        }
    }
    super::gate::trigger_backfill_if_large_gap(ctx, channel_id, received_seq, out);
}

/// 统一输出 stale/gap/协议拒绝日志，供在线丢弃与运行态回放审计。
fn log_posts_update_drop(
    frame: &WsFrame,
    channel_id: Option<ChannelId>,
    expected_seq: Option<Seq>,
    received_seq: Option<Seq>,
    accepted_count: usize,
    dropped_count: usize,
    reason: &'static str,
) {
    tracing::warn!(
        action = POSTS_UPDATE_ACTION,
        channel_id = ?channel_id,
        expected_seq = ?expected_seq.map(|seq| seq.0),
        received_seq = ?received_seq.map(|seq| seq.0),
        last_local_seq = ?expected_seq.map(|seq| seq.0.saturating_sub(1)),
        first_seq = ?received_seq.map(|seq| seq.0),
        last_seq = ?received_seq.map(|seq| seq.0),
        accepted_count,
        dropped_count,
        reason,
        trace_id = ?frame.cses_track_id(),
        "posts_update event dropped or rejected"
    );
}

static POSTS_UPDATE_HANDLER: PostsUpdateHandler = PostsUpdateHandler;
#[cfg(target_arch = "wasm32")]
pub(super) fn inventory_link_anchor() {
    std::hint::black_box(&POSTS_UPDATE_HANDLER);
}

inventory::submit! {
    WsHandlerRegistration {
        action: POSTS_UPDATE_ACTION,
        handler: &POSTS_UPDATE_HANDLER,
    }
}

#[cfg(test)]
mod tests {
    use super::parse_posts_update_batch;
    use serde_json::json;

    const CHANNEL_ID: &str = "ch000000000000000000000001";
    const OTHER_CHANNEL_ID: &str = "ch000000000000000000000002";

    /// 构造最小 canonical event，隔离 parser 测试与完整 post 投影字段。
    fn event(seq: u64, channel_id: &str) -> serde_json::Value {
        json!({
            "event_seq": seq,
            "post": {"id": format!("post{:024}", seq), "channelId": channel_id}
        })
    }

    /// 连续递增序号与同频道 post 应完整通过 canonical parser。
    #[test]
    fn parses_contiguous_events() {
        let data = json!({
            "events": [event(21, CHANNEL_ID), event(22, CHANNEL_ID)]
        });
        let parsed = parse_posts_update_batch(&data).expect("canonical batch should parse");
        assert_eq!(parsed.channel_id.as_str(), CHANNEL_ID);
        assert_eq!(
            parsed
                .events
                .iter()
                .map(|event| event.seq.0)
                .collect::<Vec<_>>(),
            [21, 22]
        );
    }

    /// 非单调 batch 必须整体拒绝，避免把乱序输入误当作可提交前缀。
    #[test]
    fn rejects_non_monotonic_events() {
        let data = json!({
            "events": [event(22, CHANNEL_ID), event(21, CHANNEL_ID)]
        });
        assert!(matches!(
            parse_posts_update_batch(&data),
            Err("non_monotonic")
        ));
    }

    /// 跨频道事件不得进入同一批原子写集。
    #[test]
    fn rejects_mixed_channels() {
        let data = json!({
            "events": [event(21, CHANNEL_ID), event(22, OTHER_CHANNEL_ID)]
        });
        assert!(matches!(
            parse_posts_update_batch(&data),
            Err("channel_mismatch")
        ));
    }

    /// 缺少每项权威 event_seq 时 fail-closed，不回退根帧或嵌套 reply 序号。
    #[test]
    fn rejects_missing_event_seq() {
        let data = json!({
            "seq": 999,
            "events": [{"post": {"id": "post000000000000000000000021", "channelId": CHANNEL_ID}}]
        });
        assert!(matches!(parse_posts_update_batch(&data), Err("malformed")));
    }
}
