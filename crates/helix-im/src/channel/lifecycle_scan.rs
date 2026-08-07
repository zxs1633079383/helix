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
            // register_channel 幂等：若已存在则不覆盖（Connected 路径可能已注册）
            let channel = self
                .state
                .channels
                .entry(channel_id)
                .or_insert_with(|| crate::channel::Channel::new(channel_id, cursor));
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
