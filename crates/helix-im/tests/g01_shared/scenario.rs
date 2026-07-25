use std::collections::BTreeSet;

use bytes::Bytes;
use helix_core::effect::{Correlation, Effect, StorageOp, TimerId};
use helix_core::tick::{AppCommand, InboundBytes, PortError, PortOutcome, ReplyBytes};
use helix_core::{EffectSink, Module, Tick};
use helix_im::interaction_frame::{
    DeliveryVm, InteractionFrameV2, MessageContentVm, MessageVm, SurfaceId, SurfaceScope,
    TypedSurfaceMutation,
};
use helix_im::module::ImConfig;
use helix_im::ImModule;
use serde_json::{json, Value};

pub const CHANNEL_ID: &str = "chfixx00000000000000000001";
pub const SERVER_ID: &str = "srvfix00000000000000000001";
pub const REQUEST_ID: &str = "g01-wdio-request-1";
pub const MARKER: &str = "g01-wdio-fixture-marker";
pub const ACTOR_ID: &str = "g01-wdio-actor";
pub const TEAM_ID: &str = "g01-wdio-company";
pub const CREATED_AT: i64 = 1_700_000_000_002;

/// 标识 G01 HTTP 终态分支，避免用布尔值混淆 sent 与 failed。
#[derive(Clone, Copy)]
pub enum Terminal {
    Sent,
    Failed,
}

/// 汇总 HTTP 分支产生的 canonical timeline Frames。
pub struct HttpScenarioTrace {
    pub sending: InteractionFrameV2,
    pub terminal: InteractionFrameV2,
    pub temporary_id: String,
}

/// 汇总 WS 分支的独立 timeline 与 dialog 结算 Frames。
pub struct WsScenarioTrace {
    pub sending: InteractionFrameV2,
    pub timeline_sent: InteractionFrameV2,
    pub dialog: InteractionFrameV2,
    pub temporary_id: String,
    pub timeline_corr: Correlation,
    pub dialog_corr: Correlation,
}

/// 通过公开 `ImModule::handle` 驱动 G01 的唯一确定性测试场景。
pub struct G01ScenarioRunner {
    module: ImModule,
    sink: EffectSink,
}

impl G01ScenarioRunner {
    /// 创建与 checked-in G01 Fixture 身份和时钟完全一致的回放实例。
    pub fn new() -> Self {
        Self {
            module: ImModule::new(ImConfig {
                api_base_url: "http://g01-wdio.invalid".to_string(),
                default_api_base_url: "http://g01-wdio.invalid".to_string(),
                auth_user_id: ACTOR_ID.to_string(),
                company_id: TEAM_ID.to_string(),
                user_name: "G01 Actor".to_string(),
                ..ImConfig::default()
            }),
            sink: EffectSink::new(),
        }
    }

    /// 回放 HTTP success/error 并在每个持久化屏障处验证禁止提前发布 Frame。
    pub fn run_http(terminal: Terminal) -> HttpScenarioTrace {
        let mut runner = Self::new();
        runner.attach_timeline();
        let sending = runner.start_sending();
        let terminal_frame = runner.finish_http(&sending, terminal);
        HttpScenarioTrace {
            sending: sending.frame,
            terminal: terminal_frame,
            temporary_id: sending.temporary_id,
        }
    }

    /// 回放 WS post 先行结算并验证迟到 HTTP 不会重复写终态。
    pub fn run_ws_echo_with_dialog() -> WsScenarioTrace {
        let mut runner = Self::new();
        runner.attach_dialog();
        runner.attach_timeline();
        let sending = runner.start_sending();

        runner.step(echo_frame(&sending.temporary_id), CREATED_AT as u64 + 20);
        assert!(
            runner.effects().iter().any(
                |effect| matches!(effect, Effect::CancelTimer { id } if *id == sending.timer_id)
            ),
            "authoritative WS echo must cancel the sending timeout"
        );
        let timeline_corr = runner.only_persist_corr_with(|ops| {
            has_table_write(ops, "message")
                && !ops
                    .iter()
                    .any(|op| matches!(op, StorageOp::Get(spec) if spec.table == "channel"))
        });
        let dialog_corr = runner.only_persist_corr_with(|ops| {
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

        runner.step(ok(timeline_corr, Bytes::new()), CREATED_AT as u64 + 21);
        assert!(
            runner.frames().is_empty(),
            "terminal PersistOk only schedules the timeline readback"
        );
        let timeline_read = runner.only_persist_corr_with(|ops| {
            ops.iter()
                .any(|op| matches!(op, StorageOp::Scan(spec) if spec.table == "message"))
        });
        runner.step(
            ok(
                timeline_read,
                rows_bytes(json!([message_row(
                    SERVER_ID,
                    &sending.temporary_id,
                    "sent",
                    1
                )])),
            ),
            CREATED_AT as u64 + 22,
        );
        let timeline_sent = runner.only_frame();

        runner.step(
            ok(
                dialog_corr,
                rows_bytes(json!([channel_row(0, MARKER, CREATED_AT + 20)])),
            ),
            CREATED_AT as u64 + 23,
        );
        let dialog = runner.only_frame();

        runner.step(
            ok(
                sending.http_corr,
                http_envelope(json!({
                    "status": "SUCCESS",
                    "data": {"id": SERVER_ID}
                })),
            ),
            CREATED_AT as u64 + 24,
        );
        assert!(
            runner
                .effects()
                .iter()
                .all(|effect| !matches!(effect, Effect::Persist { ops, .. } if has_table_write(ops, "message"))),
            "late HTTP success after WS settlement must not repeat terminal write"
        );
        assert!(
            runner.frames().is_empty(),
            "late HTTP success after WS settlement must not repeat terminal Frame"
        );

        WsScenarioTrace {
            sending: sending.frame,
            timeline_sent,
            dialog,
            temporary_id: sending.temporary_id,
            timeline_corr,
            dialog_corr,
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

    /// 建立 latest timeline attachment，但不把 setup Frame 计入 G01 发送合同。
    fn attach_timeline(&mut self) {
        self.step(
            command(
                "im_query_messages_by_channel",
                json!({
                    "channel_id": CHANNEL_ID,
                    "limit": 20,
                    "windowToken": "latest",
                }),
            ),
            1_700_000_000_000,
        );
        let local = self.only_persist_corr_with(|ops| {
            ops.iter()
                .any(|op| matches!(op, StorageOp::Scan(spec) if spec.table == "message"))
        });
        self.step(ok(local, Bytes::from_static(b"[]")), 1_700_000_000_001);
        let remote = self.only_http_corr("/posts/getLatestPost");
        self.step(
            ok(
                remote,
                http_envelope(json!({"status": "SUCCESS", "data": []})),
            ),
            1_700_000_000_002,
        );
        assert_eq!(
            surface_names(&self.only_frame()),
            BTreeSet::from(["message.timeline"])
        );
        self.step(
            command(
                "__helix_request_surface_snapshot",
                json!({
                    "surface": "message.timeline",
                    "scope": {
                        "kind": "timeline",
                        "channelId": CHANNEL_ID,
                        "windowToken": "latest"
                    },
                    "causationId": "g01-wdio-attach-1"
                }),
            ),
            1_700_000_000_003,
        );
        let _attachment_read = self.only_persist_corr_with(|ops| {
            ops.iter()
                .any(|op| matches!(op, StorageOp::Scan(spec) if spec.table == "message"))
        });
    }

    /// 建立 dialog attachment，使 WS post 可以发布左侧会话 Frame。
    fn attach_dialog(&mut self) {
        self.step(
            command("im_query_dialog_list", json!({})),
            1_699_999_999_990,
        );
        let corr = self.only_persist_corr_with(|ops| {
            ops.iter()
                .any(|op| matches!(op, StorageOp::Scan(spec) if spec.table == "channel"))
        });
        self.step(
            ok(
                corr,
                rows_bytes(json!([channel_row(0, "", CREATED_AT - 1)])),
            ),
            1_699_999_999_991,
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
                    "channel_id": CHANNEL_ID,
                    "text": MARKER,
                    "type": "TEXT",
                    "req_id": REQUEST_ID,
                }),
            ),
            1_700_000_000_004,
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
        self.step(ok(p1, Bytes::new()), 1_700_000_000_005);
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
                    &temporary_id,
                    &temporary_id,
                    "sending",
                    0
                )])),
            ),
            1_700_000_000_006,
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

    /// 跨 terminal Persist/readback 推进 HTTP 分支并返回唯一终态 Frame。
    fn finish_http(&mut self, sending: &SendingStage, terminal: Terminal) -> InteractionFrameV2 {
        match terminal {
            Terminal::Sent => self.step(
                ok(
                    sending.http_corr,
                    http_envelope(json!({
                        "status": "SUCCESS",
                        "data": {"id": SERVER_ID}
                    })),
                ),
                1_700_000_000_007,
            ),
            Terminal::Failed => self.step(http_error(sending.http_corr), 1_700_000_000_007),
        }
        assert!(
            self.frames().is_empty(),
            "HTTP completion is not render-ready before terminal persistence"
        );
        let persist = self.only_persist_corr_with(|ops| has_table_write(ops, "message"));
        self.step(ok(persist, Bytes::new()), 1_700_000_000_008);
        assert!(
            self.frames().is_empty(),
            "terminal PersistOk only schedules the terminal readback"
        );
        let read = self.only_persist_corr_with(|ops| {
            ops.iter()
                .any(|op| matches!(op, StorageOp::Scan(spec) if spec.table == "message"))
        });
        let (id, status) = match terminal {
            Terminal::Sent => (SERVER_ID, "sent"),
            Terminal::Failed => (sending.temporary_id.as_str(), "unsend"),
        };
        self.step(
            ok(
                read,
                rows_bytes(json!([message_row(
                    id,
                    &sending.temporary_id,
                    status,
                    0
                )])),
            ),
            1_700_000_000_009,
        );
        self.only_frame()
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

/// 逐字段验证 G01 timeline Frame 的 envelope、slot 和 render-ready 消息语义。
pub fn assert_timeline_contract(
    frame: &InteractionFrameV2,
    expected_delivery: &str,
    expected_id: &str,
) {
    frame.validate().expect("timeline Frame remains valid");
    assert_eq!(frame.schema_version, 2);
    assert!(!frame.transaction_id.is_empty());
    assert_eq!(frame.causation_id.as_deref(), Some(REQUEST_ID));
    assert_eq!(format!("{:?}", frame.runtime.connection), "Online");
    assert_eq!(format!("{:?}", frame.runtime.initial_sync), "Ready");
    assert_eq!(format!("{:?}", frame.runtime.recovery), "Idle");
    assert!(frame.directives.is_empty());
    assert_eq!(frame.mutations.len(), 1);
    let mutation = &frame.mutations[0];
    assert_eq!(mutation.mutation.surface, SurfaceId::MessageTimeline);
    assert_eq!(mutation.surface_epoch, 1);
    assert!(mutation.mutation.revision > 0);
    assert!(mutation.mutation.base_revision.is_some());
    assert_eq!(mutation.mutation.causation_id.as_deref(), Some(REQUEST_ID));
    let message = first_message(frame);
    assert_eq!(message.id, expected_id);
    assert_eq!(message.channel_id, CHANNEL_ID);
    assert_eq!(message.author.user_id, ACTOR_ID);
    assert_eq!(message.created_at, CREATED_AT);
    let MessageContentVm::Text { text } = &message.content else {
        panic!("G01 timeline contract requires text content")
    };
    assert_eq!(text, MARKER);
    assert!(delivery_kinds(frame).contains(expected_delivery));
}

/// 验证 WS dialog Frame 至少包含左侧会话 slot 及本次 marker/时间。
pub fn assert_dialog_contract(frame: &InteractionFrameV2) {
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
            user_id: ACTOR_ID.to_string(),
            team_id: TEAM_ID.to_string(),
        },
        "dialog Frame must target the attached actor/team slot"
    );
    let dialog = frame
        .mutations
        .iter()
        .find_map(|mutation| match &mutation.mutation.mutation {
            TypedSurfaceMutation::DialogListSnapshot { view } => Some(view),
            TypedSurfaceMutation::DialogListPatch { patch } => Some(&patch.view),
            _ => None,
        })
        .expect("dialog Frame contains a typed dialog view");
    let row = dialog
        .sections
        .iter()
        .flat_map(|section| &section.items)
        .find(|row| row.channel_id == CHANNEL_ID)
        .expect("dialog Frame contains the sent channel");
    assert_eq!(
        row.last_activity_at,
        CREATED_AT + 20,
        "WS dialog must expose the authoritative post activity; row={row:?}"
    );
    match &row.preview {
        helix_im::interaction_frame::PreviewVm::Text { text, .. } => assert_eq!(text, MARKER),
        preview => panic!("G01 dialog preview must be text, got {preview:?}"),
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

/// 构造 driver Scan 回灌使用的本地 message row。
fn message_row(id: &str, temporary_id: &str, status: &str, event_seq: u64) -> Value {
    json!({
        "id": id,
        "temporaryId": temporary_id,
        "channelId": CHANNEL_ID,
        "userId": ACTOR_ID,
        "createAt": CREATED_AT,
        "message": MARKER,
        "type": "TEXT",
        "sendStatus": status,
        "event_seq": event_seq,
        "props": {},
    })
}

/// 构造 dialog 写后 Get 使用的累计 channel row。
fn channel_row(unread_count: i64, last_post: &str, last_root_post_at: i64) -> Value {
    json!({
        "id": CHANNEL_ID,
        "team_id": TEAM_ID,
        "display_name": "G01 Flow Group",
        "type": "P",
        "unread_count": unread_count,
        "mention_count": 0,
        "urgent_count": 0,
        "mention_list": [],
        "urgent_post_list": [],
        "unread_post_id": "",
        "last_post": last_post,
        "last_post_at": last_root_post_at,
        "last_root_post_at": last_root_post_at,
    })
}

/// 构造携 temporaryId、serverId 和 event_seq 的权威 WS post。
fn echo_frame(temporary_id: &str) -> Tick {
    Tick::Inbound(InboundBytes(
        serde_json::to_vec(&json!({
            "v": 1,
            "action": "post",
            "data": {
                "id": SERVER_ID,
                "temporaryId": temporary_id,
                "channelId": CHANNEL_ID,
                "event_seq": 1,
                "create_at": CREATED_AT + 20,
                "message": MARKER,
                "simpleMessage": MARKER,
                "type": "TEXT",
                "userId": ACTOR_ID,
                "userSnapshot": {"userId": ACTOR_ID, "teamId": TEAM_ID},
                "viewers": ["all"],
                "mentions": [],
                "props": {"channel_event_seq": 1}
            },
            "broadcast": {},
            "seq": 1
        }))
        .expect("WS echo serializes")
        .into(),
    ))
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
