use std::collections::BTreeSet;

use bytes::Bytes;
use helix_core::effect::{Correlation, Effect, StorageOp, TimerId};
use helix_core::tick::{AppCommand, InboundBytes, PortError, PortOutcome, ReplyBytes};
use helix_core::{EffectSink, Module, Tick};
use helix_im::interaction_frame::{
    DeliveryVm, DialogRowVm, InteractionFrameV2, MessageContentVm, MessageVm, SurfaceId,
    SurfaceScope, TypedSurfaceMutation,
};
use helix_im::module::ImConfig;
use helix_im::ImModule;
use serde_json::{json, Value};

pub const CHANNEL_ID: &str = "chfixx00000000000000000001";
pub const SERVER_ID: &str = "srvfix00000000000000000001";
pub const REQUEST_ID: &str = "g01-wdio-request-1";
pub const MARKER: &str = "g01-wdio-fixture-marker";
pub const ACTOR_ID: &str = "g01-wdio-actor";
pub const OBSERVER_ID: &str = "g01-wdio-observer";
pub const TEAM_ID: &str = "g01-wdio-company";
pub const CREATED_AT: i64 = 1_700_000_000_002;
pub const GROUP_NAME: &str = "G01 Fixture Group";

/// 描述发送类 Gate 的确定性身份、内容和 correlation 输入。
#[derive(Clone)]
pub struct SendScenarioSpec {
    pub channel_id: &'static str,
    pub server_id: &'static str,
    pub request_id: &'static str,
    pub marker: &'static str,
    pub simple_message: &'static str,
    pub message_type: &'static str,
    pub props: Value,
    pub actor_id: &'static str,
    pub observer_id: &'static str,
    pub team_id: &'static str,
    pub created_at: i64,
    pub group_name: &'static str,
    pub api_base_url: &'static str,
}

/// 返回与既有 G01 Golden 字节完全兼容的普通文本场景参数。
pub fn g01_scenario_spec() -> SendScenarioSpec {
    SendScenarioSpec {
        channel_id: CHANNEL_ID,
        server_id: SERVER_ID,
        request_id: REQUEST_ID,
        marker: MARKER,
        simple_message: MARKER,
        message_type: "TEXT",
        props: json!({}),
        actor_id: ACTOR_ID,
        observer_id: OBSERVER_ID,
        team_id: TEAM_ID,
        created_at: CREATED_AT,
        group_name: GROUP_NAME,
        api_base_url: "http://g01-wdio.invalid",
    }
}

/// 标识发送类 Gate 的 HTTP/timeout 终态分支，避免布尔值混淆失败来源。
#[allow(dead_code)]
#[derive(Clone, Copy)]
pub enum Terminal {
    Sent,
    BusinessFailed,
    Failed,
}

/// 汇总 HTTP 分支产生的 canonical timeline Frames。
pub struct HttpScenarioTrace {
    pub group_created: InteractionFrameV2,
    pub sending: InteractionFrameV2,
    pub terminal: InteractionFrameV2,
    pub temporary_id: String,
}

/// 汇总 WS 分支的独立 timeline 与 dialog 结算 Frames。
pub struct WsScenarioTrace {
    pub group_created: InteractionFrameV2,
    pub sending: InteractionFrameV2,
    pub timeline_sent: InteractionFrameV2,
    pub dialog: InteractionFrameV2,
    pub temporary_id: String,
    pub timeline_corr: Correlation,
    pub dialog_corr: Correlation,
}

/// 汇总一次失败后重试的 HTTP 终态 Frames。
pub struct RetryHttpScenarioTrace {
    pub group_created: InteractionFrameV2,
    pub group_timeline_empty: InteractionFrameV2,
    pub initial_sending: InteractionFrameV2,
    pub initial_failed: InteractionFrameV2,
    pub retry_sending: InteractionFrameV2,
    pub retry_terminal: InteractionFrameV2,
    pub temporary_id: String,
}

/// 汇总失败重试后由 WS 先行结算的 timeline 与 dialog Frames。
pub struct RetryWsScenarioTrace {
    pub group_created: InteractionFrameV2,
    pub group_timeline_empty: InteractionFrameV2,
    pub initial_failed: InteractionFrameV2,
    pub retry_sending: InteractionFrameV2,
    pub timeline_sent: InteractionFrameV2,
    pub dialog: InteractionFrameV2,
    pub temporary_id: String,
    pub timeline_corr: Correlation,
    pub dialog_corr: Correlation,
}

/// 汇总 cold-start durable failed row 重建后的 retry Frames。
pub struct RetryRecoveryScenarioTrace {
    pub group_created: InteractionFrameV2,
    pub retry_sending: InteractionFrameV2,
    pub retry_sent: InteractionFrameV2,
    pub temporary_id: String,
}

/// 汇总冷启动本地持久事实恢复出的 timeline Frame。
#[allow(dead_code)]
pub struct RecoveryScenarioTrace {
    pub group_created: InteractionFrameV2,
    pub recovered: InteractionFrameV2,
}

/// 汇总 G04 online revoke 的可撤回与权威撤回 Frames。
pub struct RevokeWsScenarioTrace {
    pub group_created: InteractionFrameV2,
    pub revocable: InteractionFrameV2,
    pub recalled: InteractionFrameV2,
}

/// 汇总 G04 HTTP 失败分支，证明 accepted 不改变既有可见状态。
pub struct RevokeFailureScenarioTrace {
    pub group_created: InteractionFrameV2,
    pub revocable: InteractionFrameV2,
    pub emitted_frame_count: usize,
    pub persisted_write_count: usize,
}

/// 汇总 G04 authority 持久化失败后的重放收敛结果。
pub struct RevokePersistFailureScenarioTrace {
    pub group_created: InteractionFrameV2,
    pub revocable: InteractionFrameV2,
    pub recalled_after_replay: InteractionFrameV2,
}

/// 汇总 cold-start durable revoked row 重建出的 timeline Frame。
pub struct RevokeRecoveryScenarioTrace {
    pub group_created: InteractionFrameV2,
    pub recovered: InteractionFrameV2,
}

/// 通过公开 `ImModule::handle` 驱动发送类 Gate 的共享确定性测试场景。
pub struct G01ScenarioRunner {
    spec: SendScenarioSpec,
    module: ImModule,
    sink: EffectSink,
}

#[allow(dead_code)]
impl G01ScenarioRunner {
    /// 创建与 checked-in G01 Fixture 身份和时钟完全一致的回放实例。
    pub fn new() -> Self {
        Self::new_for(g01_scenario_spec())
    }

    /// 以 Gate 专属身份和消息内容创建共享发送回放实例。
    pub fn new_for(spec: SendScenarioSpec) -> Self {
        Self {
            module: ImModule::new(ImConfig {
                api_base_url: spec.api_base_url.to_string(),
                default_api_base_url: spec.api_base_url.to_string(),
                auth_user_id: spec.actor_id.to_string(),
                company_id: spec.team_id.to_string(),
                user_name: "Fixture Actor".to_string(),
                ..ImConfig::default()
            }),
            spec,
            sink: EffectSink::new(),
        }
    }

    /// 回放 HTTP success/error 并在每个持久化屏障处验证禁止提前发布 Frame。
    pub fn run_http(terminal: Terminal) -> HttpScenarioTrace {
        Self::run_http_for(g01_scenario_spec(), terminal)
    }

    /// 回放任一发送类 Gate 的 HTTP success/business failure/infrastructure failure。
    pub fn run_http_for(spec: SendScenarioSpec, terminal: Terminal) -> HttpScenarioTrace {
        let mut runner = Self::new_for(spec);
        let group_created = runner.create_group_prerequisite();
        runner.attach_timeline();
        let sending = runner.start_sending();
        let terminal_frame = runner.finish_http(&sending, terminal);
        HttpScenarioTrace {
            group_created,
            sending: sending.frame,
            terminal: terminal_frame,
            temporary_id: sending.temporary_id,
        }
    }

    /// 回放首次发送失败后，以相同 temporaryId 重试到指定 HTTP 终态。
    pub fn run_retry_http_for(
        spec: SendScenarioSpec,
        retry_terminal: Terminal,
    ) -> RetryHttpScenarioTrace {
        let mut runner = Self::new_for(spec);
        let group_created = runner.create_group_prerequisite();
        let group_timeline_empty = runner.attach_timeline();
        let initial = runner.start_sending();
        let initial_failed = runner.finish_http(&initial, Terminal::Failed);
        let retry = runner.start_retry(&initial.temporary_id, 16_000);
        let retry_terminal = runner.finish_http_at(
            &retry,
            retry_terminal,
            runner.spec.created_at as u64 + 16_003,
        );
        RetryHttpScenarioTrace {
            group_created,
            group_timeline_empty,
            initial_sending: initial.frame,
            initial_failed,
            retry_sending: retry.frame,
            retry_terminal,
            temporary_id: retry.temporary_id,
        }
    }

    /// 回放首次失败、重试、WS 先行结算以及迟到 HTTP/重复 WS 幂等分支。
    pub fn run_retry_ws_echo_with_dialog_for(spec: SendScenarioSpec) -> RetryWsScenarioTrace {
        let mut runner = Self::new_for(spec);
        let group_created = runner.create_group_prerequisite();
        let group_timeline_empty = runner.attach_timeline();
        let initial = runner.start_sending();
        let initial_failed = runner.finish_http(&initial, Terminal::Failed);
        let retry = runner.start_retry(&initial.temporary_id, 16_000);
        let (timeline_sent, dialog, timeline_corr, dialog_corr) = runner.finish_ws(&retry, 32_000);
        RetryWsScenarioTrace {
            group_created,
            group_timeline_empty,
            initial_failed,
            retry_sending: retry.frame,
            timeline_sent,
            dialog,
            temporary_id: retry.temporary_id,
            timeline_corr,
            dialog_corr,
        }
    }

    /// 从 durable failed row 冷启动重试并返回 sending 与 sent typed Frames。
    pub fn run_retry_recovery_for(spec: SendScenarioSpec) -> RetryRecoveryScenarioTrace {
        let mut runner = Self::new_for(spec);
        let group_created = runner.create_group_prerequisite();
        runner.attach_timeline();
        runner.step(
            command(
                "im_retry_send",
                json!({
                    "message_id": "tmp-g03a-recovered",
                    "req_id": runner.spec.request_id,
                }),
            ),
            runner.spec.created_at as u64 + 40_000,
        );
        let lookup = runner.only_persist_corr_with(|ops| {
            ops.iter().any(
                |op| matches!(op, StorageOp::Get(spec) if spec.table == "message" && spec.key_col == "temporary_id"),
            )
        });
        runner.step(
            ok(
                lookup,
                rows_bytes(json!([durable_failed_row(
                    &runner.spec,
                    "tmp-g03a-recovered"
                )])),
            ),
            runner.spec.created_at as u64 + 40_001,
        );
        let retry = runner.complete_retry_start("tmp-g03a-recovered", 40_002);
        let retry_terminal = runner.finish_http_at(
            &retry,
            Terminal::Sent,
            runner.spec.created_at as u64 + 40_005,
        );
        RetryRecoveryScenarioTrace {
            group_created,
            retry_sending: retry.frame,
            retry_sent: retry_terminal,
            temporary_id: retry.temporary_id,
        }
    }

    /// 回放 WS post 先行结算并验证迟到 HTTP 不会重复写终态。
    pub fn run_ws_echo_with_dialog() -> WsScenarioTrace {
        Self::run_ws_echo_with_dialog_for(g01_scenario_spec())
    }

    /// 回放任一发送类 Gate 的 WS 先行结算和迟到 HTTP 幂等分支。
    pub fn run_ws_echo_with_dialog_for(spec: SendScenarioSpec) -> WsScenarioTrace {
        let mut runner = Self::new_for(spec);
        let group_created = runner.create_group_prerequisite();
        runner.attach_timeline();
        let sending = runner.start_sending();
        let (timeline_sent, dialog, timeline_corr, dialog_corr) = runner.finish_ws(&sending, 20);

        WsScenarioTrace {
            group_created,
            sending: sending.frame,
            timeline_sent,
            dialog,
            temporary_id: sending.temporary_id,
            timeline_corr,
            dialog_corr,
        }
    }

    /// 从持久化 document/text row 冷启动恢复 timeline，证明 Frame 不依赖端侧缓存。
    pub fn run_recovery_for(spec: SendScenarioSpec) -> RecoveryScenarioTrace {
        let mut runner = Self::new_for(spec);
        let group_created = runner.create_group_prerequisite();
        runner.step(
            command(
                "im_query_messages_by_channel",
                json!({
                    "channel_id": runner.spec.channel_id,
                    "limit": 20,
                    "windowToken": "latest",
                    "req_id": runner.spec.request_id,
                }),
            ),
            runner.spec.created_at as u64 + 30,
        );
        let local = runner.only_persist_corr_with(|ops| {
            ops.iter()
                .any(|op| matches!(op, StorageOp::Scan(spec) if spec.table == "message"))
        });
        runner.step(
            ok(
                local,
                rows_bytes(json!([message_row(
                    &runner.spec,
                    runner.spec.server_id,
                    runner.spec.server_id,
                    "sent",
                    1
                )])),
            ),
            runner.spec.created_at as u64 + 31,
        );
        let recovered = if runner.frames().is_empty() {
            let remote = runner.only_http_corr("/posts/getLatestPost");
            runner.step(
                ok(
                    remote,
                    http_envelope(json!({
                        "status": "SUCCESS",
                        "data": [message_row(
                            &runner.spec,
                            runner.spec.server_id,
                            runner.spec.server_id,
                            "sent",
                            1
                        )]
                    })),
                ),
                runner.spec.created_at as u64 + 32,
            );
            if runner.frames().is_empty() {
                let cache = runner.only_persist_corr_with(|ops| !ops.is_empty());
                runner.step(ok(cache, Bytes::new()), runner.spec.created_at as u64 + 33);
            }
            runner.only_frame()
        } else {
            runner.only_frame()
        };
        RecoveryScenarioTrace {
            group_created,
            recovered,
        }
    }

    /// 回放 G04 action admission、WS authority、PersistAtomic 与 recalled timeline。
    pub fn run_revoke_ws_for(spec: SendScenarioSpec) -> RevokeWsScenarioTrace {
        let mut runner = Self::new_for(spec);
        let group_created = runner.create_group_prerequisite();
        let revocable = runner.attach_revoke_timeline(false);
        let http_corr = runner.start_revoke(&revocable);
        let recalled = runner.finish_revoke_ws(http_corr, false);
        RevokeWsScenarioTrace {
            group_created,
            revocable,
            recalled,
        }
    }

    /// 回放 G04 HTTP 基础设施失败，证明没有伪成功 Persist 或 Frame。
    pub fn run_revoke_http_failure_for(spec: SendScenarioSpec) -> RevokeFailureScenarioTrace {
        let mut runner = Self::new_for(spec);
        let group_created = runner.create_group_prerequisite();
        let revocable = runner.attach_revoke_timeline(false);
        let http_corr = runner.start_revoke(&revocable);
        runner.step(http_error(http_corr), runner.spec.created_at as u64 + 10);
        RevokeFailureScenarioTrace {
            group_created,
            revocable,
            emitted_frame_count: runner.frames().len(),
            persisted_write_count: runner
                .effects()
                .iter()
                .filter(|effect| {
                    matches!(
                        effect,
                        Effect::Persist { .. } | Effect::PersistAtomic { .. }
                    )
                })
                .count(),
        }
    }

    /// 回放 G04 authority Persist 失败并以同一 WS fact 重放成功。
    pub fn run_revoke_persist_failure_for(
        spec: SendScenarioSpec,
    ) -> RevokePersistFailureScenarioTrace {
        let mut runner = Self::new_for(spec);
        let group_created = runner.create_group_prerequisite();
        let revocable = runner.attach_revoke_timeline(false);
        let http_corr = runner.start_revoke(&revocable);
        runner.step(
            revoke_ws_frame(&runner.spec),
            runner.spec.created_at as u64 + 10,
        );
        let failed_corr =
            runner.only_persist_atomic_corr_with(|ops| has_table_write(ops, "message"));
        runner.step(
            Tick::PortReply {
                corr: failed_corr,
                outcome: PortOutcome::Err(PortError::Storage(500)),
            },
            runner.spec.created_at as u64 + 11,
        );
        assert!(
            runner.frames().is_empty(),
            "failed revoke PersistAtomic must not publish a business Frame"
        );
        runner.step(
            revoke_ws_frame(&runner.spec),
            runner.spec.created_at as u64 + 12,
        );
        let replay_corr =
            runner.only_persist_atomic_corr_with(|ops| has_table_write(ops, "message"));
        let recalled_after_replay =
            runner.complete_revoke_persist(replay_corr, runner.spec.created_at as u64 + 13);
        runner.step(
            ok(
                http_corr,
                http_envelope(json!({"status": "SUCCESS", "data": {}})),
            ),
            runner.spec.created_at as u64 + 15,
        );
        assert!(
            runner.frames().is_empty(),
            "late HTTP after replayed WS authority must not publish another Frame"
        );
        RevokePersistFailureScenarioTrace {
            group_created,
            revocable,
            recalled_after_replay,
        }
    }

    /// 从 durable revoked row 冷启动恢复 G04 recalled snapshot。
    pub fn run_revoke_recovery_for(spec: SendScenarioSpec) -> RevokeRecoveryScenarioTrace {
        let mut runner = Self::new_for(spec);
        let group_created = runner.create_group_prerequisite();
        let recovered = runner.attach_revoke_timeline(true);
        RevokeRecoveryScenarioTrace {
            group_created,
            recovered,
        }
    }

    /// 以单调注入时间推进 Tick，并只保留当前 step 的 Effect 批次。
    fn step(&mut self, tick: Tick, now_ms: u64) {
        self.sink.clear();
        self.module
            .handle(&tick, now_ms, &mut self.sink)
            .expect("G01 scenario step must succeed");
    }

    /// 暴露当前 step 的 Effect，供因果屏障和幂等断言使用。
    fn effects(&self) -> &[Effect] {
        self.sink.as_slice()
    }

    /// 先挂载空 dialog，再以双用户建群回包产出所有发送类 UC 可复用的权威 V2 前置 Frame。
    pub fn create_group_prerequisite(&mut self) -> InteractionFrameV2 {
        self.attach_empty_dialog();
        self.step(
            command(
                "im_create_channel",
                json!({
                    "type": "O",
                    "user_ids": [self.spec.observer_id],
                    "display_name": self.spec.group_name,
                    "force_create": true,
                    "req_id": format!("{}:create-group", self.spec.request_id),
                }),
            ),
            self.spec.created_at as u64 - 10,
        );
        let create_corr = self.only_http_corr("/channel/create");
        let create_body = self
            .effects()
            .iter()
            .find_map(|effect| match effect {
                Effect::Http { corr, req } if *corr == create_corr => req.body.as_ref(),
                _ => None,
            })
            .and_then(|body| serde_json::from_slice::<Value>(body.as_ref()).ok())
            .expect("create-group HTTP body remains inspectable");
        assert_eq!(
            create_body["userIds"],
            json!([self.spec.observer_id]),
            "current actor is the implicit owner; only the observer is invited"
        );
        assert_eq!(create_body["displayName"], self.spec.group_name);

        self.step(
            ok(
                create_corr,
                http_envelope(json!({
                    "status": "SUCCESS",
                    "data": {
                        "id": self.spec.channel_id,
                        "type": "P",
                        "displayName": self.spec.group_name,
                        "teamId": self.spec.team_id,
                        "createAt": self.spec.created_at - 5,
                        "owner": {"id": self.spec.actor_id, "role": "OWNER"},
                        "pictureType": "USER",
                        "picture": {"userIds": [self.spec.actor_id, self.spec.observer_id]}
                    }
                })),
            ),
            self.spec.created_at as u64 - 9,
        );
        let persisted_tables = self
            .effects()
            .iter()
            .filter_map(|effect| match effect {
                Effect::PersistFire { ops } => Some(ops.as_slice()),
                _ => None,
            })
            .flatten()
            .filter_map(|op| match op {
                StorageOp::BatchUpsert(spec) => Some(spec.table),
                StorageOp::BatchUpdate(spec) => Some(spec.table),
                _ => None,
            })
            .collect::<BTreeSet<_>>();
        assert!(
            persisted_tables.contains("channel") && persisted_tables.contains("channel_member"),
            "authoritative create reply must persist channel and member facts"
        );
        assert!(
            self.effects().iter().any(|effect| {
                let Effect::Emit { event } = effect else {
                    return false;
                };
                serde_json::from_slice::<Value>(event.0.as_ref())
                    .ok()
                    .is_some_and(|envelope| {
                        envelope["event"] == "im:channel:created"
                            && envelope["data"]["channelId"] == self.spec.channel_id
                    })
            }),
            "authoritative create reply must publish the created channel identity"
        );
        self.only_frame()
    }

    /// 通过生产 local-first query 挂载一条可撤回或已撤回 timeline row。
    fn attach_revoke_timeline(&mut self, revoked: bool) -> InteractionFrameV2 {
        self.step(
            command(
                "im_query_messages_by_channel",
                json!({
                    "channel_id": self.spec.channel_id,
                    "limit": 20,
                    "windowToken": "latest",
                    "req_id": self.spec.request_id,
                }),
            ),
            self.spec.created_at as u64,
        );
        let local = self.only_persist_corr_with(|ops| {
            ops.iter()
                .any(|op| matches!(op, StorageOp::Scan(spec) if spec.table == "message"))
        });
        self.step(
            ok(
                local,
                rows_bytes(json!([revoke_message_row(&self.spec, revoked)])),
            ),
            self.spec.created_at as u64 + 1,
        );
        if self.frames().is_empty() {
            let remote = self.only_http_corr("/posts/getLatestPost");
            self.step(
                ok(
                    remote,
                    http_envelope(json!({
                        "status": "SUCCESS",
                        "data": [revoke_message_row(&self.spec, revoked)]
                    })),
                ),
                self.spec.created_at as u64 + 2,
            );
            if self.frames().is_empty() {
                let cache = self.only_persist_corr_with(|ops| !ops.is_empty());
                self.step(ok(cache, Bytes::new()), self.spec.created_at as u64 + 3);
            }
        }
        self.only_frame()
    }

    /// 从 Helix 签发的 opaque revoke binding 构造严格 action envelope。
    fn start_revoke(&mut self, frame: &InteractionFrameV2) -> Correlation {
        let mutation = frame
            .mutations
            .first()
            .expect("revoke timeline contains one mutation");
        let action = first_message(frame)
            .actions
            .iter()
            .find(|action| action.kind == "revoke")
            .expect("revoke timeline contains its authority binding");
        assert!(action.enabled, "revoke authority binding must be live");
        let payload = serde_json::to_vec(&json!({
            "schemaVersion": 1,
            "requestId": self.spec.request_id,
            "actorId": self.spec.actor_id,
            "surface": "message.timeline",
            "scope": mutation.mutation.scope,
            "surfaceEpoch": action.surface_epoch,
            "observedRevision": mutation.mutation.revision,
            "invocation": {
                "actionRef": action.action_ref,
                "input": {}
            }
        }))
        .expect("revoke action envelope serializes");
        let action_command = helix_im::client_api::build_action_invocation_command(&payload)
            .expect("revoke action envelope is accepted by the production client API");
        self.step(
            Tick::Command(action_command),
            self.spec.created_at as u64 + 2,
        );
        assert!(
            self.frames().is_empty(),
            "AcceptedCommand/action receipt must not fabricate recalled UI"
        );
        self.only_http_corr("/posts/revoke")
    }

    /// 完成在线撤回 WS authority，并验证重复 WS 与迟到 HTTP 幂等。
    fn finish_revoke_ws(
        &mut self,
        http_corr: Correlation,
        persist_failed_once: bool,
    ) -> InteractionFrameV2 {
        self.step(
            revoke_ws_frame(&self.spec),
            self.spec.created_at as u64 + 10,
        );
        let persist_corr =
            self.only_persist_atomic_corr_with(|ops| has_table_write(ops, "message"));
        if persist_failed_once {
            self.step(
                Tick::PortReply {
                    corr: persist_corr,
                    outcome: PortOutcome::Err(PortError::Storage(500)),
                },
                self.spec.created_at as u64 + 11,
            );
            assert!(
                self.frames().is_empty(),
                "failed revoke persistence cannot publish recalled UI"
            );
            self.step(
                revoke_ws_frame(&self.spec),
                self.spec.created_at as u64 + 12,
            );
            let replay_corr =
                self.only_persist_atomic_corr_with(|ops| has_table_write(ops, "message"));
            return self.complete_revoke_persist(replay_corr, self.spec.created_at as u64 + 13);
        }
        let recalled = self.complete_revoke_persist(persist_corr, self.spec.created_at as u64 + 11);
        self.step(
            revoke_ws_frame(&self.spec),
            self.spec.created_at as u64 + 13,
        );
        assert!(
            self.effects().is_empty(),
            "duplicate revoke authority must be ignored after cursor commit"
        );
        self.step(
            ok(
                http_corr,
                http_envelope(json!({"status": "SUCCESS", "data": {}})),
            ),
            self.spec.created_at as u64 + 14,
        );
        assert!(
            self.frames().is_empty(),
            "late HTTP success must not repeat the recalled Frame"
        );
        recalled
    }

    /// 在 PersistAtomic 成功后回灌 durable row，并取得唯一 recalled Frame。
    fn complete_revoke_persist(
        &mut self,
        persist_corr: Correlation,
        now_ms: u64,
    ) -> InteractionFrameV2 {
        self.step(ok(persist_corr, Bytes::new()), now_ms);
        assert!(
            self.frames().is_empty(),
            "revoke PersistOk only schedules attached timeline readback"
        );
        let read_corr = self.only_persist_corr_with(|ops| {
            ops.iter()
                .any(|op| matches!(op, StorageOp::Scan(spec) if spec.table == "message"))
        });
        self.step(
            ok(
                read_corr,
                rows_bytes(json!([revoke_message_row(&self.spec, true)])),
            ),
            now_ms + 1,
        );
        self.only_frame()
    }

    /// 建立 latest timeline attachment，但不把 setup Frame 计入 G01 发送合同。
    fn attach_timeline(&mut self) -> InteractionFrameV2 {
        self.step(
            command(
                "im_query_messages_by_channel",
                json!({
                    "channel_id": self.spec.channel_id,
                    "limit": 20,
                    "windowToken": "latest",
                }),
            ),
            self.spec.created_at as u64 - 2,
        );
        let local = self.only_persist_corr_with(|ops| {
            ops.iter()
                .any(|op| matches!(op, StorageOp::Scan(spec) if spec.table == "message"))
        });
        self.step(
            ok(local, Bytes::from_static(b"[]")),
            self.spec.created_at as u64 - 1,
        );
        let remote = self.only_http_corr("/posts/getLatestPost");
        self.step(
            ok(
                remote,
                http_envelope(json!({"status": "SUCCESS", "data": []})),
            ),
            self.spec.created_at as u64,
        );
        let empty_timeline = self.only_frame();
        assert_eq!(
            surface_names(&empty_timeline),
            BTreeSet::from(["message.timeline"])
        );
        self.step(
            command(
                "__helix_request_surface_snapshot",
                json!({
                    "surface": "message.timeline",
                    "scope": {
                        "kind": "timeline",
                        "channelId": self.spec.channel_id,
                        "windowToken": "latest"
                    },
                    "causationId": format!("{}:attach", self.spec.request_id)
                }),
            ),
            self.spec.created_at as u64 + 1,
        );
        let _attachment_read = self.only_persist_corr_with(|ops| {
            ops.iter()
                .any(|op| matches!(op, StorageOp::Scan(spec) if spec.table == "message"))
        });
        empty_timeline
    }

    /// 以空频道快照建立 dialog attachment，使建群 Frame 本身插入首条会话。
    fn attach_empty_dialog(&mut self) {
        self.step(
            command("im_query_dialog_list", json!({})),
            self.spec.created_at as u64 - 12,
        );
        let corr = self.only_persist_corr_with(|ops| {
            ops.iter()
                .any(|op| matches!(op, StorageOp::Scan(spec) if spec.table == "channel"))
        });
        self.step(
            ok(corr, rows_bytes(json!([]))),
            self.spec.created_at as u64 - 11,
        );
        assert!(
            surface_names(&self.only_frame()).contains("message.left-dialog-list"),
            "dialog setup must attach the left-list surface"
        );
    }

    /// 推进 optimistic P1、readback、sending Frame、HTTP 和 timeout 同步可见的阶段。
    fn start_sending(&mut self) -> SendingStage {
        self.step(
            command(
                "im_send_message",
                json!({
                    "channel_id": self.spec.channel_id,
                    "text": self.spec.marker,
                    "type": self.spec.message_type,
                    "props": self.spec.props.clone(),
                    "req_id": self.spec.request_id,
                }),
            ),
            self.spec.created_at as u64 + 2,
        );
        assert!(
            self.effects()
                .iter()
                .all(|effect| !matches!(effect, Effect::Http { .. })),
            "posts/create must wait for optimistic Persist"
        );
        assert!(
            self.frames().is_empty(),
            "send command cannot fabricate a render-ready row before readback"
        );
        let temporary_id = self.temporary_id();
        let p1 = self.only_persist_corr_with(|ops| has_table_write(ops, "message"));
        self.step(ok(p1, Bytes::new()), self.spec.created_at as u64 + 3);
        assert!(
            self.frames().is_empty(),
            "optimistic PersistOk only schedules the attached timeline readback"
        );
        let read = self.only_persist_corr_with(|ops| {
            ops.iter()
                .any(|op| matches!(op, StorageOp::Scan(spec) if spec.table == "message"))
        });
        self.step(
            ok(
                read,
                rows_bytes(json!([message_row(
                    &self.spec,
                    &temporary_id,
                    &temporary_id,
                    "sending",
                    0
                )])),
            ),
            self.spec.created_at as u64 + 4,
        );
        let frame = self.only_frame();
        let http_corr = self.only_http_corr("/posts/create");
        let timer_id = self
            .effects()
            .iter()
            .find_map(|effect| match effect {
                Effect::ScheduleTimer { id, after_ms } if *after_ms == 15_000 => Some(*id),
                _ => None,
            })
            .expect("sending stage schedules the 15 second timeout");
        SendingStage {
            temporary_id,
            http_corr,
            timer_id,
            frame,
        }
    }

    /// 从 failed pending send 进入重试 Persist/readback/HTTP 阶段。
    fn start_retry(&mut self, temporary_id: &str, offset_ms: u64) -> SendingStage {
        self.step(
            command(
                "im_retry_send",
                json!({
                    "message_id": temporary_id,
                    "req_id": self.spec.request_id,
                }),
            ),
            self.spec.created_at as u64 + offset_ms,
        );
        assert!(
            self.effects()
                .iter()
                .all(|effect| !matches!(effect, Effect::Http { .. })),
            "retry posts/create must wait for its sending PersistOk"
        );
        assert!(
            self.frames().is_empty(),
            "retry command cannot fabricate a typed Frame before readback"
        );
        self.complete_retry_start(temporary_id, offset_ms + 1)
    }

    /// 在当前 step 的 retry Persist 后完成 readback，并捕获新的 HTTP/timer correlation。
    fn complete_retry_start(&mut self, temporary_id: &str, offset_ms: u64) -> SendingStage {
        let persist = self.only_persist_corr_with(|ops| has_table_write(ops, "message"));
        self.step(
            ok(persist, Bytes::new()),
            self.spec.created_at as u64 + offset_ms,
        );
        assert!(
            self.frames().is_empty(),
            "retry PersistOk only schedules the attached timeline readback"
        );
        let read = self.only_persist_corr_with(|ops| {
            ops.iter()
                .any(|op| matches!(op, StorageOp::Scan(spec) if spec.table == "message"))
        });
        self.step(
            ok(
                read,
                rows_bytes(json!([message_row(
                    &self.spec,
                    temporary_id,
                    temporary_id,
                    "sending",
                    0
                )])),
            ),
            self.spec.created_at as u64 + offset_ms + 1,
        );
        let frame = self.only_frame();
        let http_corr = self.only_http_corr("/posts/create");
        let timer_id = self
            .effects()
            .iter()
            .find_map(|effect| match effect {
                Effect::ScheduleTimer { id, after_ms } if *after_ms == 15_000 => Some(*id),
                _ => None,
            })
            .expect("retry sending stage schedules a fresh timeout");
        SendingStage {
            temporary_id: temporary_id.to_string(),
            http_corr,
            timer_id,
            frame,
        }
    }

    /// 跨 terminal Persist/readback 推进 HTTP 分支并返回唯一终态 Frame。
    fn finish_http(&mut self, sending: &SendingStage, terminal: Terminal) -> InteractionFrameV2 {
        self.finish_http_at(sending, terminal, self.spec.created_at as u64 + 5)
    }

    /// 从指定单调时间推进 HTTP/timeout、终态 Persist 与 timeline readback。
    fn finish_http_at(
        &mut self,
        sending: &SendingStage,
        terminal: Terminal,
        terminal_at_ms: u64,
    ) -> InteractionFrameV2 {
        match terminal {
            Terminal::Sent => self.step(
                ok(
                    sending.http_corr,
                    http_envelope(json!({
                        "status": "SUCCESS",
                        "data": {"id": self.spec.server_id}
                    })),
                ),
                terminal_at_ms,
            ),
            Terminal::BusinessFailed => {
                self.step(
                    ok(
                        sending.http_corr,
                        http_envelope(json!({
                            "status": "ERROR",
                            "message": "document access rejected"
                        })),
                    ),
                    terminal_at_ms,
                );
                assert!(
                    self.effects()
                        .iter()
                        .all(|effect| !matches!(effect, Effect::Persist { ops, .. } if has_table_write(ops, "message"))),
                    "business rejection must wait for the armed timeout instead of fabricating sent"
                );
                self.step(Tick::Timer(sending.timer_id), terminal_at_ms + 15_000);
            }
            Terminal::Failed => self.step(http_error(sending.http_corr), terminal_at_ms),
        }
        assert!(
            self.frames().is_empty(),
            "HTTP completion is not render-ready before terminal persistence"
        );
        let persist = self.only_persist_corr_with(|ops| has_table_write(ops, "message"));
        self.step(ok(persist, Bytes::new()), terminal_at_ms + 15_001);
        assert!(
            self.frames().is_empty(),
            "terminal PersistOk only schedules the terminal readback"
        );
        let read = self.only_persist_corr_with(|ops| {
            ops.iter()
                .any(|op| matches!(op, StorageOp::Scan(spec) if spec.table == "message"))
        });
        let (id, status) = match terminal {
            Terminal::Sent => (self.spec.server_id, "sent"),
            Terminal::BusinessFailed | Terminal::Failed => {
                (sending.temporary_id.as_str(), "unsend")
            }
        };
        self.step(
            ok(
                read,
                rows_bytes(json!([message_row(
                    &self.spec,
                    id,
                    &sending.temporary_id,
                    status,
                    0
                )])),
            ),
            terminal_at_ms + 15_002,
        );
        self.only_frame()
    }

    /// 以 WS authority 结算当前发送阶段，并验证迟到 HTTP 与重复 WS 不重写终态。
    fn finish_ws(
        &mut self,
        sending: &SendingStage,
        offset_ms: u64,
    ) -> (
        InteractionFrameV2,
        InteractionFrameV2,
        Correlation,
        Correlation,
    ) {
        self.step(
            echo_frame(&self.spec, &sending.temporary_id),
            self.spec.created_at as u64 + offset_ms,
        );
        assert!(
            self.effects().iter().any(
                |effect| matches!(effect, Effect::CancelTimer { id } if *id == sending.timer_id)
            ),
            "authoritative WS echo must cancel the sending timeout"
        );
        let timeline_corr = self.only_persist_corr_with(|ops| {
            has_table_write(ops, "message")
                && !ops
                    .iter()
                    .any(|op| matches!(op, StorageOp::Get(spec) if spec.table == "channel"))
        });
        let dialog_corr = self.only_persist_corr_with(|ops| {
            ops.iter()
                .any(|op| matches!(op, StorageOp::GuardedBump(spec) if spec.table == "channel"))
                && ops
                    .iter()
                    .any(|op| matches!(op, StorageOp::Get(spec) if spec.table == "channel"))
        });
        assert_ne!(
            timeline_corr, dialog_corr,
            "timeline and dialog must retain independent correlation barriers"
        );

        self.step(
            ok(timeline_corr, Bytes::new()),
            self.spec.created_at as u64 + offset_ms + 1,
        );
        assert!(
            self.frames().is_empty(),
            "terminal PersistOk only schedules the timeline readback"
        );
        let timeline_read = self.only_persist_corr_with(|ops| {
            ops.iter()
                .any(|op| matches!(op, StorageOp::Scan(spec) if spec.table == "message"))
        });
        self.step(
            ok(
                timeline_read,
                rows_bytes(json!([message_row(
                    &self.spec,
                    self.spec.server_id,
                    &sending.temporary_id,
                    "sent",
                    1
                )])),
            ),
            self.spec.created_at as u64 + offset_ms + 2,
        );
        let timeline_sent = self.only_frame();

        self.step(
            ok(
                dialog_corr,
                rows_bytes(json!([channel_row(
                    &self.spec,
                    0,
                    self.spec.marker,
                    self.spec.created_at + 20
                )])),
            ),
            self.spec.created_at as u64 + offset_ms + 3,
        );
        let dialog = self.only_frame();

        self.step(
            ok(
                sending.http_corr,
                http_envelope(json!({
                    "status": "SUCCESS",
                    "data": {"id": self.spec.server_id}
                })),
            ),
            self.spec.created_at as u64 + offset_ms + 4,
        );
        assert!(
            self.effects()
                .iter()
                .all(|effect| !matches!(effect, Effect::Persist { ops, .. } if has_table_write(ops, "message"))),
            "late HTTP success after WS settlement must not repeat terminal write"
        );
        assert!(
            self.frames().is_empty(),
            "late HTTP success after WS settlement must not repeat terminal Frame"
        );
        self.step(
            echo_frame(&self.spec, &sending.temporary_id),
            self.spec.created_at as u64 + offset_ms + 5,
        );
        assert!(
            self.effects()
                .iter()
                .all(|effect| !matches!(effect, Effect::Persist { ops, .. } if has_table_write(ops, "message"))),
            "duplicate authoritative WS echo must not repeat terminal message writes"
        );
        assert!(
            self.frames().is_empty(),
            "duplicate authoritative WS echo must not repeat terminal timeline Frame"
        );
        (timeline_sent, dialog, timeline_corr, dialog_corr)
    }

    /// 按 endpoint suffix 提取唯一 HTTP correlation，避免依赖 Effect 排序。
    fn only_http_corr(&self, suffix: &str) -> Correlation {
        let mut matches = self.effects().iter().filter_map(|effect| match effect {
            Effect::Http { corr, req } if req.url.ends_with(suffix) => Some(*corr),
            _ => None,
        });
        let corr = matches.next().expect("expected one matching HTTP effect");
        assert!(
            matches.next().is_none(),
            "HTTP correlation must be unambiguous for {suffix}"
        );
        corr
    }

    /// 按 StorageOp 语义提取唯一 Persist correlation。
    fn only_persist_corr_with(&self, predicate: impl Fn(&[StorageOp]) -> bool) -> Correlation {
        let mut matches = self.effects().iter().filter_map(|effect| match effect {
            Effect::Persist { corr, ops } if predicate(ops) => Some(*corr),
            _ => None,
        });
        let corr = matches
            .next()
            .expect("expected one matching Persist effect");
        assert!(
            matches.next().is_none(),
            "Persist correlation must be causally unambiguous"
        );
        corr
    }

    /// 按 StorageOp 语义提取唯一 PersistAtomic correlation。
    fn only_persist_atomic_corr_with(
        &self,
        predicate: impl Fn(&[StorageOp]) -> bool,
    ) -> Correlation {
        let mut matches = self.effects().iter().filter_map(|effect| match effect {
            Effect::PersistAtomic { corr, ops } if predicate(ops) => Some(*corr),
            _ => None,
        });
        let corr = matches
            .next()
            .expect("expected one matching PersistAtomic effect");
        assert!(
            matches.next().is_none(),
            "PersistAtomic correlation must be causally unambiguous"
        );
        corr
    }

    /// 解码当前 step 发布的所有完整 InteractionFrameV2。
    fn frames(&self) -> Vec<InteractionFrameV2> {
        self.effects()
            .iter()
            .filter_map(|effect| {
                let Effect::Emit { event } = effect else {
                    return None;
                };
                let envelope: Value = serde_json::from_slice(event.0.as_ref()).ok()?;
                (envelope["event"] == "im:interaction:frame")
                    .then(|| serde_json::to_vec(&envelope["data"]).ok())
                    .flatten()
            })
            .map(|bytes| {
                InteractionFrameV2::decode(&bytes)
                    .expect("Helix producer output must decode and validate as V2")
            })
            .collect()
    }

    /// 取得当前 step 的唯一 Frame，防止 Fixture 静默丢失批内输出。
    fn only_frame(&self) -> InteractionFrameV2 {
        let mut frames = self.frames().into_iter();
        let frame = frames.next().expect("expected one InteractionFrameV2");
        assert!(
            frames.next().is_none(),
            "scenario step must expose one unambiguous Frame"
        );
        frame
    }

    /// 从 `im:post:sending` 信封读取 Helix 注入的 temporaryId。
    fn temporary_id(&self) -> String {
        self.effects()
            .iter()
            .find_map(|effect| {
                let Effect::Emit { event } = effect else {
                    return None;
                };
                let envelope: Value = serde_json::from_slice(event.0.as_ref()).ok()?;
                (envelope["event"] == "im:post:sending")
                    .then(|| envelope["data"]["temporaryId"].as_str().map(str::to_string))
                    .flatten()
            })
            .expect("send command exposes its Helix temporary identity")
    }
}

struct SendingStage {
    temporary_id: String,
    http_corr: Correlation,
    timer_id: TimerId,
    frame: InteractionFrameV2,
}

/// 返回 Frame 中涉及的 canonical surface 集合。
pub fn surface_names(frame: &InteractionFrameV2) -> BTreeSet<&'static str> {
    frame
        .mutations
        .iter()
        .map(|mutation| mutation.mutation.surface.as_str())
        .collect()
}

/// 返回 timeline Frame 内的全部 delivery 类型。
pub fn delivery_kinds(frame: &InteractionFrameV2) -> BTreeSet<&str> {
    frame
        .mutations
        .iter()
        .filter_map(|mutation| match &mutation.mutation.mutation {
            TypedSurfaceMutation::TimelineSnapshot { view } => Some(&view.items),
            TypedSurfaceMutation::TimelinePatch { patch } => Some(&patch.view.items),
            _ => None,
        })
        .flatten()
        .map(|item| match item.delivery {
            DeliveryVm::Sending => "sending",
            DeliveryVm::Sent => "sent",
            DeliveryVm::Failed { .. } => "failed",
            DeliveryVm::Uploading { .. } => "uploading",
            DeliveryVm::Recalled => "recalled",
        })
        .collect()
}

/// 返回 timeline Frame 中 G01 对应的第一条消息。
pub fn first_message(frame: &InteractionFrameV2) -> &MessageVm {
    frame
        .mutations
        .iter()
        .find_map(|mutation| match &mutation.mutation.mutation {
            TypedSurfaceMutation::TimelineSnapshot { view } => view.items.first(),
            TypedSurfaceMutation::TimelinePatch { patch } => patch.view.items.first(),
            _ => None,
        })
        .expect("G01 timeline Frame contains its message")
}

/// 返回 dialog Frame 中指定 channel 的 render-ready 行。
pub fn dialog_row<'a>(frame: &'a InteractionFrameV2, channel_id: &str) -> &'a DialogRowVm {
    frame
        .mutations
        .iter()
        .find_map(|mutation| match &mutation.mutation.mutation {
            TypedSurfaceMutation::DialogListSnapshot { view } => Some(view),
            TypedSurfaceMutation::DialogListPatch { patch } => Some(&patch.view),
            _ => None,
        })
        .into_iter()
        .flat_map(|view| &view.sections)
        .flat_map(|section| &section.items)
        .find(|row| row.channel_id == channel_id)
        .expect("dialog Frame contains the requested channel")
}

/// 逐字段验证 G01 timeline Frame 的 envelope、slot 和 render-ready 消息语义。
pub fn assert_timeline_contract(
    frame: &InteractionFrameV2,
    expected_delivery: &str,
    expected_id: &str,
) {
    let spec = g01_scenario_spec();
    assert_timeline_contract_for(
        frame,
        &spec,
        expected_delivery,
        expected_id,
        Some(spec.request_id),
    );
    let MessageContentVm::Text { text } = &first_message(frame).content else {
        panic!("G01 timeline contract requires text content")
    };
    assert_eq!(text, MARKER);
}

/// 逐字段验证任一发送类 Gate 的 timeline envelope、slot 与消息公共语义。
pub fn assert_timeline_contract_for(
    frame: &InteractionFrameV2,
    spec: &SendScenarioSpec,
    expected_delivery: &str,
    expected_id: &str,
    expected_causation: Option<&str>,
) {
    frame.validate().expect("timeline Frame remains valid");
    assert_eq!(frame.schema_version, 2);
    assert!(!frame.transaction_id.is_empty());
    assert_eq!(frame.causation_id.as_deref(), expected_causation);
    assert_eq!(format!("{:?}", frame.runtime.connection), "Online");
    assert_eq!(format!("{:?}", frame.runtime.initial_sync), "Ready");
    assert_eq!(format!("{:?}", frame.runtime.recovery), "Idle");
    assert!(frame.directives.is_empty());
    assert_eq!(frame.mutations.len(), 1);
    let mutation = &frame.mutations[0];
    assert_eq!(mutation.mutation.surface, SurfaceId::MessageTimeline);
    assert_eq!(mutation.surface_epoch, 1);
    assert!(mutation.mutation.revision > 0);
    assert_eq!(
        mutation.mutation.base_revision.is_some(),
        expected_causation.is_some(),
        "send refresh is a patch while cold recovery starts with a snapshot"
    );
    assert_eq!(
        mutation.mutation.causation_id.as_deref(),
        expected_causation
    );
    let message = first_message(frame);
    assert_eq!(message.id, expected_id);
    assert_eq!(message.channel_id, spec.channel_id);
    assert_eq!(message.author.user_id, spec.actor_id);
    assert_eq!(message.created_at, spec.created_at);
    assert!(delivery_kinds(frame).contains(expected_delivery));
}

/// 逐字段验证建群前置 Frame 已把同一权威 channel 插入双用户左侧会话。
pub fn assert_group_created_contract(frame: &InteractionFrameV2) {
    let spec = g01_scenario_spec();
    assert_group_created_contract_for(frame, &spec);
}

/// 逐字段验证任一 Gate 的建群前置 Frame 使用其权威 channel 与双用户 roster。
pub fn assert_group_created_contract_for(frame: &InteractionFrameV2, spec: &SendScenarioSpec) {
    frame.validate().expect("group-created Frame remains valid");
    assert_eq!(frame.schema_version, 2);
    assert!(!frame.transaction_id.is_empty());
    assert!(frame.directives.is_empty());
    assert_eq!(
        surface_names(frame),
        BTreeSet::from(["message.left-dialog-list"]),
        "zero-unread group creation must publish only its dialog mutation"
    );
    let dialog_mutation = frame
        .mutations
        .iter()
        .find(|mutation| mutation.mutation.surface == SurfaceId::MessageLeftDialogList)
        .expect("group-created Frame contains its dialog mutation");
    assert_eq!(
        dialog_mutation.mutation.scope,
        SurfaceScope::User {
            user_id: spec.actor_id.to_string(),
            team_id: spec.team_id.to_string(),
        }
    );
    let row = dialog_row(frame, spec.channel_id);
    assert_eq!(row.channel_id, spec.channel_id);
    assert_eq!(row.title.text, spec.group_name);
    assert_eq!(format!("{:?}", row.channel_kind), "Group");
    assert_eq!(row.member_count, 2);
    assert_eq!(
        row.avatar.user_ids,
        vec![spec.actor_id.to_string(), spec.observer_id.to_string()],
        "group member-grid preserves the deterministic two-user roster"
    );
    assert_eq!(row.last_activity_at, spec.created_at - 5);
    assert_eq!(format!("{:?}", row.preview), "Empty");
    assert_eq!(row.lifecycle.state, "active");
}

/// 验证 WS dialog Frame 至少包含左侧会话 slot 及本次 marker/时间。
pub fn assert_dialog_contract(frame: &InteractionFrameV2) {
    let spec = g01_scenario_spec();
    assert_dialog_contract_for(frame, &spec);
}

/// 验证任一发送类 Gate 的 WS dialog Frame 包含其 channel、摘要与权威时间。
pub fn assert_dialog_contract_for(frame: &InteractionFrameV2, spec: &SendScenarioSpec) {
    frame.validate().expect("dialog Frame remains valid");
    assert_eq!(frame.schema_version, 2);
    assert!(!frame.transaction_id.is_empty());
    assert!(frame.directives.is_empty());
    assert!(
        surface_names(frame).contains("message.left-dialog-list"),
        "WS contract must include the dialog-list mutation"
    );
    let dialog_mutation = frame
        .mutations
        .iter()
        .find(|mutation| mutation.mutation.surface == SurfaceId::MessageLeftDialogList)
        .expect("dialog Frame contains its typed mutation");
    assert_eq!(
        dialog_mutation.mutation.scope,
        SurfaceScope::User {
            user_id: spec.actor_id.to_string(),
            team_id: spec.team_id.to_string(),
        },
        "dialog Frame must target the attached actor/team slot"
    );
    let row = dialog_row(frame, spec.channel_id);
    assert_eq!(
        row.last_activity_at,
        spec.created_at + 20,
        "WS dialog must expose the authoritative post activity; row={row:?}"
    );
    match (&row.preview, spec.message_type) {
        (helix_im::interaction_frame::PreviewVm::Text { text, .. }, "TEXT") => {
            assert_eq!(text, spec.simple_message)
        }
        (helix_im::interaction_frame::PreviewVm::Document { title, .. }, "DOCUMENT") => {
            assert_eq!(title, spec.simple_message)
        }
        (preview, kind) => panic!("send dialog preview mismatch for {kind}, got {preview:?}"),
    }
}

/// 构造公开 command ingress，保持测试只走 `Tick::Command`。
fn command(name: &'static str, payload: Value) -> Tick {
    Tick::Command(AppCommand::new(
        name,
        serde_json::to_vec(&payload).expect("command payload serializes"),
    ))
}

/// 构造成功 PortReply。
fn ok(corr: Correlation, bytes: Bytes) -> Tick {
    Tick::PortReply {
        corr,
        outcome: PortOutcome::Ok(ReplyBytes(bytes)),
    }
}

/// 构造 posts/create HTTP 500 回报。
fn http_error(corr: Correlation) -> Tick {
    Tick::PortReply {
        corr,
        outcome: PortOutcome::Err(PortError::Http(500)),
    }
}

/// 按 host driver 的 ADR-007 形状封装 HTTP body。
fn http_envelope(body: Value) -> Bytes {
    let raw = serde_json::to_vec(&body).expect("HTTP body serializes");
    Bytes::from(
        serde_json::to_vec(&json!({
            "status": 200,
            "headers": {},
            "body": base64(&raw),
        }))
        .expect("HTTP envelope serializes"),
    )
}

/// 将 JSON rows 编码为 host storage reply bytes。
fn rows_bytes(rows: Value) -> Bytes {
    Bytes::from(serde_json::to_vec(&rows).expect("storage rows serialize"))
}

/// 构造 driver Scan 回灌使用的 Gate 专属本地 message row。
fn message_row(
    spec: &SendScenarioSpec,
    id: &str,
    temporary_id: &str,
    status: &str,
    event_seq: u64,
) -> Value {
    json!({
        "id": id,
        "temporaryId": temporary_id,
        "channelId": spec.channel_id,
        "userId": spec.actor_id,
        "createAt": spec.created_at,
        "message": spec.marker,
        "type": spec.message_type,
        "sendStatus": status,
        "event_seq": event_seq,
        "props": spec.props,
    })
}

/// 构造 G04 local-first query 与撤回 readback 使用的 render-ready message row。
fn revoke_message_row(spec: &SendScenarioSpec, revoked: bool) -> Value {
    json!({
        "id": spec.server_id,
        "temporaryId": spec.server_id,
        "channelId": spec.channel_id,
        "userId": spec.actor_id,
        "createAt": spec.created_at,
        "message": spec.marker,
        "type": spec.message_type,
        "sendStatus": "sent",
        "event_seq": if revoked { 1 } else { 0 },
        "revoke": revoked,
        "props": spec.props,
        "actionCapabilities": {
            "canRevoke": !revoked,
            "canReply": !revoked,
            "canForward": !revoked,
            "canReact": !revoked
        }
    })
}

/// 构造冷启动 retry lookup 使用的 durable failed message row。
fn durable_failed_row(spec: &SendScenarioSpec, temporary_id: &str) -> Value {
    json!({
        "temporary_id": temporary_id,
        "id": "",
        "channel_id": spec.channel_id,
        "user_id": spec.actor_id,
        "type": spec.message_type,
        "message": spec.marker,
        "props": serde_json::to_string(&spec.props).expect("scenario props serialize"),
        "viewers": "[]",
        "mentions": "[]",
        "user_snapshot": serde_json::to_string(&json!({
            "userId": spec.actor_id,
            "teamId": spec.team_id,
        }))
        .expect("scenario user snapshot serializes"),
        "topic": "",
        "reply_id": "",
        "reply_root_id": "",
        "reply_first_level_id": "",
        "create_at": spec.created_at,
        "send_status": "unsend",
    })
}

/// 构造 dialog 写后 Get 使用的累计 channel row。
fn channel_row(
    spec: &SendScenarioSpec,
    unread_count: i64,
    last_post: &str,
    last_root_post_at: i64,
) -> Value {
    json!({
        "id": spec.channel_id,
        "team_id": spec.team_id,
        "display_name": spec.group_name,
        "type": "P",
        "unread_count": unread_count,
        "mention_count": 0,
        "urgent_count": 0,
        "mention_list": [],
        "urgent_post_list": [],
        "unread_post_id": "",
        "last_post": last_post,
        "last_message_type": spec.message_type,
        "last_post_at": last_root_post_at,
        "last_root_post_at": last_root_post_at,
    })
}

/// 构造携 temporaryId、serverId 和 event_seq 的权威 WS post。
fn echo_frame(spec: &SendScenarioSpec, temporary_id: &str) -> Tick {
    Tick::Inbound(InboundBytes(
        serde_json::to_vec(&json!({
            "v": 1,
            "action": "post",
            "data": {
                "id": spec.server_id,
                "temporaryId": temporary_id,
                "channelId": spec.channel_id,
                "event_seq": 1,
                "create_at": spec.created_at + 20,
                "message": spec.marker,
                "simpleMessage": spec.simple_message,
                "type": spec.message_type,
                "userId": spec.actor_id,
                "userSnapshot": {"userId": spec.actor_id, "teamId": spec.team_id},
                "viewers": ["all"],
                "mentions": [],
                "props": merge_channel_event_seq(&spec.props)
            },
            "broadcast": {},
            "seq": 1
        }))
        .expect("WS echo serializes")
        .into(),
    ))
}

/// 构造 Go `posts_update` 的权威撤回 fact，cursor 固定推进到 1。
fn revoke_ws_frame(spec: &SendScenarioSpec) -> Tick {
    Tick::Inbound(InboundBytes(Bytes::from(
        serde_json::to_vec(&json!({
            "action": "posts_update",
            "data": {
                "event_seq": 1,
                "channelId": spec.channel_id,
                "actor_id": spec.actor_id,
                "occurred_at": spec.created_at + 10,
                "posts": [{
                    "id": spec.server_id,
                    "channelId": spec.channel_id,
                    "userId": spec.actor_id,
                    "message": spec.marker,
                    "type": spec.message_type,
                    "revoke": true,
                    "props": spec.props
                }]
            }
        }))
        .expect("revoke WS frame serializes"),
    )))
}

/// 合并 WS authority 序号而不改写 Gate 自身的 props 业务字段。
fn merge_channel_event_seq(props: &Value) -> Value {
    let mut merged = props.as_object().cloned().unwrap_or_default();
    merged.insert("channel_event_seq".to_string(), json!(1));
    Value::Object(merged)
}

/// 判断一组 StorageOp 是否包含指定表的业务写。
fn has_table_write(ops: &[StorageOp], table: &str) -> bool {
    ops.iter().any(|op| match op {
        StorageOp::BatchUpsert(spec) => spec.table == table,
        StorageOp::BatchUpdate(spec) => spec.table == table,
        StorageOp::GuardedBump(spec) => spec.table == table,
        _ => false,
    })
}

/// 以固定字母表编码测试 HTTP 信封，避免新增运行时依赖。
fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut encoded = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = u32::from(chunk[0]);
        let b1 = u32::from(chunk.get(1).copied().unwrap_or_default());
        let b2 = u32::from(chunk.get(2).copied().unwrap_or_default());
        let value = (b0 << 16) | (b1 << 8) | b2;
        encoded.push(TABLE[((value >> 18) & 63) as usize] as char);
        encoded.push(TABLE[((value >> 12) & 63) as usize] as char);
        encoded.push(if chunk.len() > 1 {
            TABLE[((value >> 6) & 63) as usize] as char
        } else {
            '='
        });
        encoded.push(if chunk.len() > 2 {
            TABLE[(value & 63) as usize] as char
        } else {
            '='
        });
    }
    encoded
}
