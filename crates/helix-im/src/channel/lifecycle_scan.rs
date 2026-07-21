//! `handle_scan_reply`（E4 / S2：on_start Scan{channel_event_cursor} 的 PortReply 处理）。
//!
//! 从 `lifecycle.rs` 拆出（structure-gate 300 行硬顶·many-small-files·零行为变更）。
//! 仍是 `ImModule` 的 inherent 方法（Rust 多 impl 块），与 `lifecycle.rs` 其余生命周期
//! 方法共享同一类型；调用的 `emit_proactive_resync` 留在 `lifecycle.rs`。

use crate::error::ImError;
use crate::module::ImModule;
use crate::state::ChannelId;
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

        // 若 hello 早于 scan 回包，首个 bootstrap increment 已经以空 cursors 发出。
        // scan 载入 cursor 后补发一次带真实游标的 `/channels/load/increment`，让 Go 回推
        // `increment_channel` 分片，恢复 UC-4.1 的 `im:channel:increment` 契约面。
        if self.state.conn == crate::state::ConnState::Connected
            && self.state.pending_increment_bootstrap_after_scan
        {
            self.state.pending_increment_bootstrap_after_scan = false;
            if !self.state.channels.is_empty() {
                let conn_id = self.state.connection_id.clone();
                let cursors = self.channel_cursors();
                let inc_corr = self.alloc_corr_internal();
                out.push(crate::acl::to_effect::increment_http_trigger(
                    &self.config.api_base_url,
                    &cursors,
                    inc_corr,
                    conn_id.as_deref(),
                ));
            }
        }

        // 仅在已连接时触发 proactive resync（复用 Connected 路径逻辑）
        // 若此时尚未连接，Connected 事件到来后会自行 resync
        if self.state.conn == crate::state::ConnState::Connected {
            self.emit_proactive_resync(out);
        }

        Ok(())
    }
}
