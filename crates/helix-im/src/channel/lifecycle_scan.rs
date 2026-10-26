//! `handle_scan_reply` 与 `handle_channel_projection_scan_reply`（E4 / S2：on_start
//! 的 cursor/channel 双扫描 PortReply 处理）。
//!
//! 从 `lifecycle.rs` 拆出（structure-gate 300 行硬顶·many-small-files·零行为变更）。
//! 仍是 `ImModule` 的 inherent 方法（Rust 多 impl 块），与 `lifecycle.rs` 其余生命周期
//! 方法共享同一类型；调用的 `emit_proactive_resync` 留在 `lifecycle.rs`。

use crate::error::ImError;
use crate::module::ImModule;
use crate::state::{ChannelId, Seq};
use helix_core::EffectSink;

impl ImModule {
    /// 解析本地 message 水位，并用 hello 时冻结的 cursor 快照触发一次 increment HTTP。
    pub(crate) fn handle_increment_message_timestamp_scan_reply(
        &mut self,
        connection_id: Option<String>,
        cursors: Vec<(ChannelId, Seq)>,
        outcome: &helix_core::tick::PortOutcome,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        if self.state.conn != crate::state::ConnState::Connected
            || self.state.connection_id.as_deref() != connection_id.as_deref()
        {
            tracing::debug!(
                expected_connection_id = ?connection_id,
                active_connection_id = ?self.state.connection_id,
                "ignoring stale increment message timestamp scan reply"
            );
            return Ok(());
        }

        let timestamp = match outcome {
            helix_core::tick::PortOutcome::Ok(reply) => {
                parse_latest_message_create_at(reply.0.as_ref())
            }
            helix_core::tick::PortOutcome::Err(error) => {
                tracing::warn!(
                    error = ?error,
                    "message watermark scan failed; increment timestamp falls back to zero"
                );
                0
            }
        };
        if self.state.increment_page_supported {
            self.start_increment_pull(timestamp, cursors, out);
            return Ok(());
        }
        let increment_corr = self.alloc_corr_internal();
        out.push(crate::acl::to_effect::increment_http_trigger(
            &self.config.api_base_url,
            timestamp,
            &cursors,
            increment_corr,
            self.state.connection_id.as_deref(),
        ));
        Ok(())
    }

    /// E4 / S2：处理 on_start Scan{channel_event_cursor} 的 PortReply
    ///
    /// `channel_event_cursor` 是 advance_cursor 写入的实际表（channel_id TEXT PK,
    /// last_event_seq INTEGER, updated_at INTEGER）。engine_loop 将 rows 序列化为 JSON
    /// 数组 `[{"channel_id": "<id>", "last_event_seq": N, "updated_at": T}, ...]`。
    ///
    /// channel_id 是 26 字符 Id26 字符串（IM 业务约定，见 acl/to_effect.rs::advance_cursor）。
    /// last_event_seq 即该 channel 的 cursor 水位。
    pub(crate) fn handle_scan_reply(
        &mut self,
        reply: &helix_core::tick::ReplyBytes,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        // engine_loop 约定：rows → JSON 数组 [{"col": val}, ...]
        // channel_event_cursor 表：channel_id TEXT PK, last_event_seq INTEGER（cursor）+
        // terminal_event_seq INTEGER（只表示 type7 closed tombstone）。两项由 terminal
        // PersistAtomic 同批写入；scan 恢复时 marker 超过 cursor 即视作损坏并 fail closed。
        let rows: Vec<serde_json::Value> = if reply.0.is_empty() {
            vec![]
        } else {
            serde_json::from_slice(reply.0.as_ref()).unwrap_or_else(|e| {
                tracing::warn!(
                    error = %e,
                    raw = %String::from_utf8_lossy(reply.0.as_ref()),
                    "scan reply JSON parse failed, treating as empty rows"
                );
                vec![]
            })
        };

        for row in &rows {
            // channel_id 是 TEXT（26 字符 id），解析回 ChannelId（零信任）
            let channel_id = match row["channel_id"].as_str().and_then(ChannelId::from_str) {
                Some(id) => id,
                None => {
                    tracing::warn!(
                        channel_id = ?row["channel_id"],
                        "scan row channel_id not a valid 26-char channel_id, skipping"
                    );
                    continue;
                }
            };
            let cursor = match row["last_event_seq"].as_i64() {
                Some(v) if v >= 0 => v as u64,
                _ => 0u64,
            };
            let terminal_seq = match row["terminal_event_seq"].as_i64() {
                Some(v) if v > 0 => Some(crate::state::Seq(v as u64)),
                Some(0) | None => None,
                _ => {
                    tracing::warn!(
                        channel_id = channel_id.as_str(),
                        terminal_event_seq = ?row["terminal_event_seq"],
                        "terminal tombstone marker is invalid; suppressing marker"
                    );
                    None
                }
            };
            // Connected/WS 可能先注册 cursor=0；扫描权威必须单调恢复已持久水位。
            let channel = self
                .state
                .channels
                .entry(channel_id)
                .or_insert_with(|| crate::channel::Channel::new(channel_id, cursor));
            restore_scanned_cursor(channel, Seq(cursor));
            if let Some(terminal_seq) = terminal_seq {
                if !channel.restore_terminal(terminal_seq) {
                    tracing::error!(
                        channel_id = channel_id.as_str(),
                        cursor = channel.cursor.value().0,
                        terminal_event_seq = terminal_seq.0,
                        "terminal tombstone is ahead of cursor; channel remains non-terminal and is not safe to resync"
                    );
                    // Keep the cursor entry available, but an impossible terminal marker must not allow a
                    // normal resync to revive a potentially closed channel. Mark it terminal in-memory at
                    // the current cursor only after this fail-closed boundary is reported.
                    channel.terminal_event_seq = Some(channel.cursor.value());
                }
            }
        }

        tracing::info!(
            channel_count = rows.len(),
            "scan_corr resolved, loaded {} channels from cursor store",
            rows.len()
        );

        // 先读取 channel 投影，再决定哪些 cursor 可以进入补偿队列。
        self.request_channel_projection_scan(out);

        Ok(())
    }

    /// 处理 channel 投影扫描，删除/关闭频道直接进入本地 terminal，避免重启后继续补偿。
    pub(crate) fn handle_channel_projection_scan_reply(
        &mut self,
        reply: &helix_core::tick::ReplyBytes,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let rows: Vec<serde_json::Value> = if reply.0.is_empty() {
            vec![]
        } else {
            match serde_json::from_slice(reply.0.as_ref()) {
                Ok(rows) => rows,
                Err(error) => {
                    tracing::warn!(
                        %error,
                        raw = %String::from_utf8_lossy(reply.0.as_ref()),
                        "channel projection scan JSON parse failed; startup sync remains fail-closed"
                    );
                    return Ok(());
                }
            }
        };

        let mut terminal_count = 0usize;
        for row in &rows {
            self.render_scope.observe_channel(row);
            let Some(channel_id) = row
                .get("id")
                .or_else(|| row.get("channel_id"))
                .and_then(serde_json::Value::as_str)
                .and_then(ChannelId::from_str)
            else {
                continue;
            };
            let delete_at = row
                .get("delete_at")
                .or_else(|| row.get("deleteAt"))
                .and_then(as_i64)
                .unwrap_or(0);
            let is_remove = row
                .get("is_remove")
                .or_else(|| row.get("isRemove"))
                .and_then(as_bool)
                .unwrap_or(false);
            if delete_at <= 0 && !is_remove {
                continue;
            }
            let projection_seq = row
                .get("last_event_seq")
                .or_else(|| row.get("lastEventSeq"))
                .and_then(as_i64)
                .unwrap_or(0)
                .max(0) as u64;
            if let Some(channel) = self.state.channels.get_mut(&channel_id) {
                let terminal_seq = Seq(projection_seq.max(channel.cursor.value().0));
                channel.mark_projection_terminal(terminal_seq);
                terminal_count += 1;
            }
        }

        tracing::info!(
            channel_count = rows.len(),
            terminal_count,
            "channel projection scan resolved; deleted channels excluded from sync"
        );
        self.state.startup_channel_projection_ready = true;
        self.finish_startup_scan(out);
        Ok(())
    }
}

/// 从 scoped max 回报中读取最大 create_at；空行、畸形值和负数都回退为 0。
fn parse_latest_message_create_at(bytes: &[u8]) -> i64 {
    if bytes.is_empty() {
        return 0;
    }
    let rows: Vec<serde_json::Value> = match serde_json::from_slice(bytes) {
        Ok(rows) => rows,
        Err(error) => {
            tracing::warn!(
                %error,
                reason = "invalid_scoped_message_watermark",
                "message watermark scan JSON parse failed; using timestamp zero"
            );
            return 0;
        }
    };

    rows.iter()
        .filter_map(|row| row.get("create_at").and_then(serde_json::Value::as_i64))
        .filter(|timestamp| *timestamp >= 0)
        .max()
        .unwrap_or(0)
}

/// 把启动扫描的持久游标单调合并到抢先注册的频道，禁止回退运行期已推进水位。
fn restore_scanned_cursor(channel: &mut crate::channel::Channel, persisted: Seq) {
    channel.cursor.try_advance(persisted);
}

/// 读取 channel 投影中的整数列，拒绝布尔/浮点等非契约值。
fn as_i64(value: &serde_json::Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_u64().and_then(|v| i64::try_from(v).ok()))
}

/// 读取 channel 投影中的删除标记，兼容 SQLite 整数与 JSON 布尔形态。
fn as_bool(value: &serde_json::Value) -> Option<bool> {
    value
        .as_bool()
        .or_else(|| value.as_i64().map(|v| v != 0))
        .or_else(|| {
            value.as_str().and_then(|v| match v {
                "1" | "true" | "TRUE" => Some(true),
                "0" | "false" | "FALSE" => Some(false),
                _ => None,
            })
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// WS 抢先注册的零游标必须被启动扫描恢复，否则后续接龙事件会永久落入 gap。
    #[test]
    fn startup_scan_restores_cursor_for_pre_registered_channel() {
        let channel_id =
            ChannelId::from_str("12345678901234567890123456").expect("valid channel id");
        let mut channel = crate::channel::Channel::new(channel_id, 0);

        restore_scanned_cursor(&mut channel, Seq(19));

        assert_eq!(channel.cursor.value(), Seq(19));
    }

    /// 启动扫描晚到时不得把运行期已提交 cursor 回退到旧水位。
    #[test]
    fn startup_scan_never_regresses_live_cursor() {
        let channel_id =
            ChannelId::from_str("12345678901234567890123456").expect("valid channel id");
        let mut channel = crate::channel::Channel::new(channel_id, 23);

        restore_scanned_cursor(&mut channel, Seq(19));

        assert_eq!(channel.cursor.value(), Seq(23));
    }

    /// 水位只认 scoped channel 回报中的最大 create_at；update_at 更大也不能抬高边界。
    #[test]
    fn message_watermark_uses_max_create_at_across_scoped_channel_rows() {
        let reply = br#"[
            {"channel_id":"ch-a","create_at":1700,"update_at":9900},
            {"channel_id":"ch-b","create_at":2300,"update_at":1200}
        ]"#;

        assert_eq!(parse_latest_message_create_at(reply), 2300);
    }
}
