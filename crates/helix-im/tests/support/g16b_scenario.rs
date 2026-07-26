use bytes::Bytes;
use helix_core::effect::{Effect, StorageOp};
use helix_core::tick::{InboundBytes, PortError, PortOutcome};
use helix_core::Correlation;
use helix_im::interaction_frame::{
    InteractionFrameV2, SurfaceId, SurfaceScope, TypedSurfaceMutation,
};
use serde_json::{json, Value};

#[path = "../common.rs"]
mod common;
use common::{command_json, port_reply_ok, port_reply_with_body, Harness};

pub const CHANNEL_ID: &str = "chfixx00000000000000000016";
pub const LEAVING_USER_ID: &str = "g16b-leaving";
pub const REMAINING_USER_ID: &str = "g16b-remaining";
pub const EVENT_SEQ: u64 = 44;

/// 保存 G16b attach 快照、durable roster patch 与关联屏障。
pub struct G16bLeaveTrace {
    pub initial: InteractionFrameV2,
    pub terminal: InteractionFrameV2,
    pub persist_corr: Correlation,
    pub readback_corr: Correlation,
    pub duplicate_effect_count: usize,
}

/// 通过生产 `ImModule::handle` 回放 member leave WS authority。
pub struct G16bScenarioRunner {
    harness: Harness,
}

impl G16bScenarioRunner {
    /// 建立固定 viewer identity 与确定性 correlation 空间。
    pub fn new() -> Self {
        Self {
            harness: Harness::new_with_identity(&[], REMAINING_USER_ID, "g16b-company", "", "", ""),
        }
    }

    /// 回放 attach、WS leave、PersistOk、权威 readback 与 duplicate suppression。
    pub fn run_leave() -> G16bLeaveTrace {
        let mut runner = Self::new();
        let initial = runner.attach_roster(initial_rows());
        runner.ingest_leave();
        let persist_corr = runner.only_persist_atomic_corr();
        assert!(
            runner.frames().is_empty(),
            "G16b cannot publish a Frame before PersistOk"
        );
        runner
            .harness
            .step(port_reply_ok(persist_corr.raw()), 1_700_001_610_003);
        let readback_corr = runner.only_roster_scan_corr();
        assert!(
            runner.frames().is_empty(),
            "G16b must read committed roster facts before publishing"
        );
        runner.harness.step(
            port_reply_with_body(readback_corr.raw(), rows_bytes(remaining_rows())),
            1_700_001_610_004,
        );
        let terminal = runner.only_roster_frame();
        runner.ingest_leave();
        let duplicate_effect_count = runner.harness.sink.as_slice().len();
        G16bLeaveTrace {
            initial,
            terminal,
            persist_corr,
            readback_corr,
            duplicate_effect_count,
        }
    }

    /// 注入 storage failure 并返回失败后可见 Frame 与重放写数量。
    pub fn run_persist_failure() -> (usize, usize) {
        let mut runner = Self::new();
        runner.attach_roster(initial_rows());
        runner.ingest_leave();
        let persist_corr = runner.only_persist_atomic_corr();
        runner.harness.step(
            helix_core::Tick::PortReply {
                corr: persist_corr,
                outcome: PortOutcome::Err(PortError::Storage(16)),
            },
            1_700_001_610_003,
        );
        let frame_count = runner.frames().len();
        runner.ingest_leave();
        let replay_count = runner
            .harness
            .sink
            .as_slice()
            .iter()
            .filter(|effect| matches!(effect, Effect::PersistAtomic { .. }))
            .count();
        (frame_count, replay_count)
    }

    /// 从 durable remaining-member rows 重建 cold attach snapshot。
    pub fn run_recovery() -> InteractionFrameV2 {
        let mut runner = Self::new();
        runner.attach_roster(remaining_rows())
    }

    /// attach member-roster 并用给定 durable rows 完成首次 snapshot。
    fn attach_roster(&mut self, rows: Value) -> InteractionFrameV2 {
        self.harness.step(
            command_json(
                "__helix_request_surface_snapshot",
                json!({
                    "surface":"message.member-roster",
                    "scope":{
                        "kind":"memberRoster",
                        "channelId":CHANNEL_ID,
                        "cursor":"initial"
                    }
                }),
            ),
            1_700_001_610_000,
        );
        let corr = self.only_roster_scan_corr();
        self.harness.step(
            port_reply_with_body(corr.raw(), rows_bytes(rows)),
            1_700_001_610_001,
        );
        self.only_roster_frame()
    }

    /// 注入带 event_seq 的 canonical channel_member_update leave authority。
    fn ingest_leave(&mut self) {
        self.harness.step(
            helix_core::Tick::Inbound(InboundBytes(Bytes::from(
                serde_json::to_vec(&json!({
                    "action":"channel_member_update",
                    "data":{
                        "id":CHANNEL_ID,
                        "type":"P",
                        "event_seq":EVENT_SEQ,
                        "memberChange":{"leave":[{"id":LEAVING_USER_ID}]}
                    }
                }))
                .expect("static G16b inbound serializes"),
            ))),
            1_700_001_610_002,
        );
    }

    /// 提取当前批次唯一 PersistAtomic correlation。
    fn only_persist_atomic_corr(&self) -> Correlation {
        only_corr(self.harness.sink.as_slice(), |effect| {
            matches!(effect, Effect::PersistAtomic { .. })
        })
    }

    /// 提取当前批次唯一 channel_member Scan correlation。
    fn only_roster_scan_corr(&self) -> Correlation {
        only_corr(self.harness.sink.as_slice(), |effect| {
            matches!(
                effect,
                Effect::Persist { ops, .. }
                    if ops.iter().any(
                        |op| matches!(op, StorageOp::Scan(spec) if spec.table == "channel_member")
                    )
            )
        })
    }

    /// 解码当前批次全部 InteractionFrameV2。
    fn frames(&self) -> Vec<InteractionFrameV2> {
        decode_frames(self.harness.sink.as_slice())
    }

    /// 提取当前批次唯一 member-roster Frame。
    fn only_roster_frame(&self) -> InteractionFrameV2 {
        let mut frames = self.frames().into_iter().filter(|frame| {
            frame
                .mutations
                .iter()
                .any(|mutation| mutation.mutation.surface == SurfaceId::MessageMemberRoster)
        });
        let frame = frames.next().expect("one G16b member-roster Frame");
        assert!(frames.next().is_none());
        frame
    }
}

/// 逐字段验证 roster Frame 的 envelope、scope、revision、成员与空 action 合同。
pub fn assert_roster_contract(
    frame: &InteractionFrameV2,
    expected_users: &[&str],
    expected_patch: bool,
) {
    frame.validate().expect("G16b roster Frame validates");
    assert_eq!(frame.schema_version, 2);
    assert_eq!(frame.mutations.len(), 1);
    assert!(frame.directives.is_empty());
    let mutation = &frame.mutations[0];
    assert_eq!(mutation.mutation.surface, SurfaceId::MessageMemberRoster);
    assert_eq!(
        mutation.mutation.scope,
        SurfaceScope::MemberRoster {
            channel_id: CHANNEL_ID.to_string(),
            cursor: "initial".to_string(),
        }
    );
    assert_eq!(
        mutation.mutation.revision,
        if expected_patch { 2 } else { 1 }
    );
    assert_eq!(mutation.mutation.base_revision, expected_patch.then_some(1));
    let view = match &mutation.mutation.mutation {
        TypedSurfaceMutation::MemberRosterSnapshot { view } => {
            assert!(!expected_patch);
            view
        }
        TypedSurfaceMutation::MemberRosterPatch { patch } => {
            assert!(expected_patch);
            &patch.view
        }
        other => panic!("expected G16b roster mutation, got {other:?}"),
    };
    let users = view
        .items
        .iter()
        .map(|item| item.user.user_id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(users, expected_users);
    assert_eq!(view.member_count as usize, expected_users.len());
    assert!(view.items.iter().all(|item| {
        item.membership_state == helix_im::interaction_frame::MembershipState::Active
            && item.actions.iter().all(|action| !action.enabled)
    }));
}

/// 提取 Effect 批次中唯一匹配的 correlation。
fn only_corr(effects: &[Effect], predicate: impl Fn(&Effect) -> bool) -> Correlation {
    let mut correlations = effects
        .iter()
        .filter(|effect| predicate(effect))
        .filter_map(|effect| match effect {
            Effect::Persist { corr, .. } | Effect::PersistAtomic { corr, .. } => Some(*corr),
            _ => None,
        });
    let corr = correlations.next().expect("matching G16b correlation");
    assert!(correlations.next().is_none());
    corr
}

/// 解码 Effect::Emit 中的完整 InteractionFrameV2，忽略兼容 domain events。
fn decode_frames(effects: &[Effect]) -> Vec<InteractionFrameV2> {
    effects
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
        .map(|bytes| InteractionFrameV2::decode(&bytes).expect("producer emits valid G16b V2"))
        .collect()
}

/// 构造 attach 前的两名 active member durable rows。
fn initial_rows() -> Value {
    json!([
        member_row(LEAVING_USER_ID, "MEMBER"),
        member_row(REMAINING_USER_ID, "ADMIN")
    ])
}

/// 构造 leave 持久化后的单成员 durable rows。
fn remaining_rows() -> Value {
    json!([member_row(REMAINING_USER_ID, "ADMIN")])
}

/// 构造 channel_member storage row。
fn member_row(user_id: &str, role: &str) -> Value {
    json!({
        "channel_id":CHANNEL_ID,
        "user_id":user_id,
        "team_id":"g16b-company",
        "role":role,
        "nick_name":user_id,
        "joined_at":1
    })
}

/// 编码 storage scan 使用的 JSON row array。
fn rows_bytes(value: Value) -> Bytes {
    Bytes::from(serde_json::to_vec(&value).expect("static G16b storage rows serialize"))
}
