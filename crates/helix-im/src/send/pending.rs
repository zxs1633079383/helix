//! PendingSend 聚合（发送对账的一致性边界）
//!
//! ## C006 收敛保证
//!
//! - echo 先到 → CancelTimer 让 15s 永不 fire
//! - timer 先到 → 标 UnSend；迟到 echo 仍按 temporaryId 覆盖回 Sent
//! - 两种次序终态一致，纯状态机确定性保证，不依赖时序
//!
//! ## 源码印证（2026-06-08 实测）
//!
//! - ON CONFLICT(temporary_id) DO UPDATE ... —— temporaryId 作 PK
//! - echo/sync 对账走同一对账点（reconcile）
//! - 15s 超时无自动补发（on_timeout 不产出 Http effect）

use crate::state::{SendStatus, ServerId, TemporaryId};
use helix_core::effect::{SqlValue, StorageOp, UpsertSpec};
use helix_core::{Correlation, Effect, EffectSink, TimerId};
use serde_json::Value;

mod persistence;
pub use persistence::{
    optimistic_message_persist_op, send_status_persist_op, upload_progress_persist_op,
};

/// 一次发送 attempt 的 timeline 因果回读地址。
///
/// window 与 causation 必须整体复制：只复制其中一个会让 action retry 的后续事件
/// 退回 `latest` 或失去 request confirmation。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TimelineReadbackContext {
    pub window_token: Option<String>,
    pub causation_id: Option<String>,
}

/// 待对账的单条消息发送记录
pub struct PendingSend {
    /// 聚合根标识（UI 注入，非 helix-core / helix-im 生成）
    pub temporary_id: TemporaryId,
    pub status: SendStatus,
    /// 关联的 15s 超时令牌（CancelTimer 用）
    pub timeout_timer: TimerId,
    /// 用于写入数据库的 Persist corr（可选，发送时可能尚未落库）
    pub persist_corr: Option<Correlation>,
    /// 发送时的 connectionId（M1 重连去重维度）。
    ///
    /// 重连后服务端 `increment` 会把本会话已发但未收 echo 的 post 整批补回——届时按
    /// `temporary_id`（HashMap key）天然去重不双份；本字段记录「这条是哪个 connectionId 下发的」，
    /// 让重连后能判定「跨会话补回 vs 本会话 echo」（None = 发送时尚未握手，无 connectionId）。
    pub connection_id: Option<String>,
    /// 调用方 request causation 与 action 授权 retry 的原始 timeline window。
    ///
    /// 普通发送与 legacy 命令保持 `None`，readback 按 `latest`；action retry 必须在整个
    /// progress / failure / posts.create / WS echo 生命周期内保留原 window，不能只用于首帧。
    pub timeline_readback: TimelineReadbackContext,
    /// rich/file 上传完成后才真正发 posts/create；普通文本无此缓存。
    pub body: Option<Value>,
    /// `posts/create` 已经出站的单飞闸。普通文本必须先把 P1 的真实 timeline 事件
    /// 发给已 attach 的 surface，随后才允许这一跳 HTTP；上传链和 retry 也复用此闸，
    /// 防止重复的 projector 回包把同一 temporary_id 再次发出。
    pub http_started: bool,
    /// 显式 retry 的 HTTP admission 后必须强制远端读回，避免服务端幂等确认不再广播 echo 时二次超时。
    pub authoritative_readback_after_http: bool,
    /// 当前显式 retry 已完成的权威回读重试次数；只允许有界退避，避免读请求风暴。
    pub authoritative_readback_attempt: u8,
    /// 当前在途的权威回读退避 timer；命中 WS/HTTP authority 时必须显式取消。
    pub authoritative_readback_timer: Option<TimerId>,
    /// 仍待成功的上传数；归零后才发 posts/create。
    pub remaining_uploads: usize,
    /// 上传失败闸。失败时 send status 同步进入 UnSend 供公开 VM 暴露 retry；
    /// 本字段继续区分“重试媒体阶段”与“重试 posts/create”。
    pub upload_failed: bool,
}

/// 权威回读最多额外发起五次查询；总退避小于发送 15s 终态 timer。
pub(crate) const AUTHORITATIVE_READBACK_MAX_ATTEMPTS: u8 = 5;

/// 根据 1-based attempt 选择有限指数退避，给 Go 异步 Pulsar 落库留出传播时间。
pub(crate) fn authoritative_readback_backoff_ms(attempt: u8) -> u64 {
    match attempt {
        1 => 100,
        2 => 250,
        3 => 500,
        4 => 1_000,
        _ => 2_000,
    }
}

impl PendingSend {
    /// 创建新的待对账记录（Local 状态）。
    ///
    /// `connection_id` = 发送时刻的握手 connectionId（M1 维度，None = 未握手）。
    pub fn new(
        temporary_id: TemporaryId,
        timeout_timer: TimerId,
        connection_id: Option<String>,
    ) -> Self {
        Self {
            temporary_id,
            status: SendStatus::Local,
            timeout_timer,
            persist_corr: None,
            connection_id,
            timeline_readback: TimelineReadbackContext::default(),
            body: None,
            http_started: false,
            authoritative_readback_after_http: false,
            authoritative_readback_attempt: 0,
            authoritative_readback_timer: None,
            remaining_uploads: 0,
            upload_failed: false,
        }
    }

    /// echo 或 sync 对账到达（temporaryId 值级覆盖）。
    ///
    /// echo 与 sync 走完全相同 Effect 序列（源码印证）：
    /// 1. Persist{ON CONFLICT(temporary_id) 覆盖 id=serverId, status=sent}
    /// 2. CancelTimer{timeout_timer}（撤 15s 超时）
    pub fn reconcile(&mut self, server_id: ServerId, corr: Correlation, fx: &mut EffectSink) {
        // 吐 Persist（ON CONFLICT temporaryId 覆盖为 server_id + Sent）
        // MAJ-5: table 和 conflict_key 使用 &'static str 常量，避免热路径堆分配
        fx.push(Effect::Persist {
            corr,
            ops: vec![StorageOp::BatchUpsert(UpsertSpec {
                version_column: None,
                update_guard: None,
                table: "message",
                rows: vec![vec![
                    (
                        "temporary_id".to_string(),
                        SqlValue::Text(self.temporary_id.0.clone()),
                    ),
                    // msgId 是 26 字符 base32 → Text（非 Integer），同 channelId（ADR-009）
                    (
                        "id".to_string(),
                        SqlValue::Text(server_id.as_str().to_string()),
                    ),
                    (
                        "send_status".to_string(),
                        SqlValue::Text("sent".to_string()),
                    ),
                ]],
                // conflict_key 的值（"temporary_id"）由 ACL-1 提供，core 不知道含义
                conflict_key: Some("temporary_id"),
                exclude_from_update: Vec::new(),
            })],
        });

        // 吐 CancelTimer（撤 15s 超时）
        fx.push(Effect::CancelTimer {
            id: self.timeout_timer,
        });

        self.status = SendStatus::Sent;
    }

    /// F1：HTTP 200 `{status:SUCCESS}` 但响应无 server post id 时的兜底对账。
    ///
    /// 与 [`reconcile`] 区别：**不覆写 `id`**（无 serverId 可用），只把乐观行
    /// `send_status='sent'`（ON CONFLICT temporary_id）+ 撤 15s 超时 + 推进 `Sent`。
    /// serverId 由后续 history / WS echo 按 temporary_id 幂等补（后到 `reconcile` 覆盖同行无害）。
    pub fn mark_sent_pending_server_id(&mut self, corr: Correlation, fx: &mut EffectSink) {
        fx.push(Effect::Persist {
            corr,
            ops: vec![StorageOp::BatchUpsert(UpsertSpec {
                version_column: None,
                update_guard: None,
                table: "message",
                rows: vec![vec![
                    (
                        "temporary_id".to_string(),
                        SqlValue::Text(self.temporary_id.0.clone()),
                    ),
                    (
                        "send_status".to_string(),
                        SqlValue::Text("sent".to_string()),
                    ),
                ]],
                conflict_key: Some("temporary_id"),
                exclude_from_update: Vec::new(),
            })],
        });

        fx.push(Effect::CancelTimer {
            id: self.timeout_timer,
        });

        self.status = SendStatus::Sent;
    }

    /// HTTP 发送失败立即终结 → 标 UnSend + 撤 15s 超时。
    ///
    /// 与 [`on_timeout`] 写同一个 DB 终态（`send_status='unsend'`），但不等待墙钟；
    /// UC-1.4 的失败态由真实 HTTP PortError 立即驱动。
    pub fn mark_failed_immediately(&mut self, corr: Correlation, fx: &mut EffectSink) -> bool {
        // 幂等：已对账或已失败的终态不重复写。
        if self.status == SendStatus::Sent || self.status == SendStatus::UnSend {
            return false;
        }

        self.status = SendStatus::UnSend;
        fx.push(Effect::Persist {
            corr,
            ops: vec![StorageOp::BatchUpsert(UpsertSpec {
                version_column: None,
                update_guard: None,
                table: "message",
                rows: vec![vec![
                    (
                        "temporary_id".to_string(),
                        SqlValue::Text(self.temporary_id.0.clone()),
                    ),
                    (
                        "send_status".to_string(),
                        SqlValue::Text("unsend".to_string()),
                    ),
                ]],
                conflict_key: Some("temporary_id"),
                exclude_from_update: Vec::new(),
            })],
        });
        fx.push(Effect::CancelTimer {
            id: self.timeout_timer,
        });
        true
    }

    /// 15s 超时触发 → 标 UnSend（无自动补发）。
    ///
    /// C006：若 echo 迟到（status 已为 Sent），幂等忽略。
    pub fn on_timeout(&mut self, fx: &mut EffectSink) {
        if self.upload_failed {
            return;
        }
        // 幂等：已对账（Sent）或已标记 UnSend 的，重复 timer fire 忽略
        if self.status == SendStatus::Sent || self.status == SendStatus::UnSend {
            return;
        }

        self.persist_unsend(fx);
    }

    /// Persist retryable unsent state before releasing this send attempt.
    fn persist_unsend(&mut self, fx: &mut EffectSink) {
        self.status = SendStatus::UnSend;

        // PersistFire：batch_update send_status = unsend（幂等）
        fx.push(Effect::PersistFire {
            ops: vec![StorageOp::BatchUpsert(UpsertSpec {
                version_column: None,
                update_guard: None,
                table: "message",
                rows: vec![vec![
                    (
                        "temporary_id".to_string(),
                        SqlValue::Text(self.temporary_id.0.clone()),
                    ),
                    (
                        "send_status".to_string(),
                        SqlValue::Text("unsend".to_string()),
                    ),
                ]],
                conflict_key: Some("temporary_id"),
                exclude_from_update: Vec::new(),
            })],
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 退避序列必须单调且总时长留在 15s 发送终态之前。
    #[test]
    fn authoritative_readback_backoff_is_bounded() {
        let delays = (1..=AUTHORITATIVE_READBACK_MAX_ATTEMPTS)
            .map(authoritative_readback_backoff_ms)
            .collect::<Vec<_>>();
        assert_eq!(delays, vec![100, 250, 500, 1_000, 2_000]);
        assert!(delays.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(delays.iter().sum::<u64>() < 15_000);
    }
}

pub use crate::send::upload_props::props_persist_op;
