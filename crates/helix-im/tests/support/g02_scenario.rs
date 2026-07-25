use helix_im::interaction_frame::{
    InteractionFrameV2, MessageContentVm, SurfaceDataVm,
};
use serde_json::json;

#[path = "../g01_shared/scenario.rs"]
#[allow(dead_code)]
mod shared;

#[allow(unused_imports)]
pub use shared::{
    assert_dialog_contract_for, assert_group_created_contract_for,
    assert_timeline_contract_for, delivery_kinds, dialog_row, first_message,
    surface_names, HttpScenarioTrace, RecoveryScenarioTrace, SendScenarioSpec, Terminal,
    WsScenarioTrace,
};

pub const CHANNEL_ID: &str = "chdocx00000000000000000001";
pub const SERVER_ID: &str = "srvdoc00000000000000000001";
pub const REQUEST_ID: &str = "g02-wdio-request-1";
pub const DOCUMENT_URL: &str =
    "http://doc-pre.cses7.com/doc/6a6036184de8e1a30ef73c15";
pub const DOCUMENT_TITLE: &str = "打开文档";
pub const MARKER: &str =
    "【文档】 http://doc-pre.cses7.com/doc/6a6036184de8e1a30ef73c15";
pub const ACTOR_ID: &str = "g02-wdio-actor";
pub const OBSERVER_ID: &str = "g02-wdio-observer";
pub const TEAM_ID: &str = "g02-wdio-company";
pub const CREATED_AT: i64 = 1_700_000_100_002;
pub const GROUP_NAME: &str = "G02 Fixture Group";

/// 返回 G02 固定 URL 文档卡片的唯一确定性场景参数。
pub fn g02_scenario_spec() -> SendScenarioSpec {
    SendScenarioSpec {
        channel_id: CHANNEL_ID,
        server_id: SERVER_ID,
        request_id: REQUEST_ID,
        marker: MARKER,
        message_type: "DOCUMENT",
        props: json!({
            "document": {
                "version": 1,
                "url": DOCUMENT_URL,
                "title": DOCUMENT_TITLE,
                "placeholder": true
            }
        }),
        actor_id: ACTOR_ID,
        observer_id: OBSERVER_ID,
        team_id: TEAM_ID,
        created_at: CREATED_AT,
        group_name: GROUP_NAME,
        api_base_url: "http://g02-wdio.invalid",
    }
}

/// 为 G02 提供共享发送 ScenarioRunner 的轻量领域入口。
pub struct G02ScenarioRunner;

#[allow(dead_code)]
impl G02ScenarioRunner {
    /// 回放 G02 HTTP success/business failure/infrastructure failure。
    pub fn run_http(terminal: Terminal) -> HttpScenarioTrace {
        shared::G01ScenarioRunner::run_http_for(g02_scenario_spec(), terminal)
    }

    /// 回放 G02 WS 先行、迟到 HTTP 与重复 WS 的幂等分支。
    pub fn run_ws_echo_with_dialog() -> WsScenarioTrace {
        shared::G01ScenarioRunner::run_ws_echo_with_dialog_for(g02_scenario_spec())
    }

    /// 从本地持久 document row 恢复 typed timeline Frame。
    pub fn run_recovery() -> RecoveryScenarioTrace {
        shared::G01ScenarioRunner::run_recovery_for(g02_scenario_spec())
    }
}

/// 逐字段验证 G02 timeline 的 typed Document 内容与固定 URL 合同。
pub fn assert_document_timeline_contract(
    frame: &InteractionFrameV2,
    expected_delivery: &str,
    expected_id: &str,
) {
    let spec = g02_scenario_spec();
    assert_timeline_contract_for(
        frame,
        &spec,
        expected_delivery,
        expected_id,
        Some(spec.request_id),
    );
    assert_document_content(frame, expected_id);
}

/// 逐字段验证冷启动 query Frame 的 typed Document 内容，query causation 可为空。
#[allow(dead_code)]
pub fn assert_recovered_document_timeline_contract(
    frame: &InteractionFrameV2,
    expected_id: &str,
) {
    let spec = g02_scenario_spec();
    assert_timeline_contract_for(frame, &spec, "sent", expected_id, None);
    assert_document_content(frame, expected_id);
}

/// 校验 Document VM 的固定 URL、标题和 producer-authored access 字段。
fn assert_document_content(frame: &InteractionFrameV2, expected_id: &str) {
    let MessageContentVm::Document {
        document_id,
        title,
        summary,
        access,
    } = &first_message(frame).content
    else {
        panic!("G02 timeline contract requires typed document content")
    };
    assert_eq!(document_id, expected_id);
    assert_eq!(title, DOCUMENT_TITLE);
    assert_eq!(summary, DOCUMENT_URL);
    let SurfaceDataVm::Record(access) = access else {
        panic!("G02 document access must preserve producer-authored props")
    };
    assert_eq!(
        access.get("url"),
        Some(&SurfaceDataVm::String(DOCUMENT_URL.to_string()))
    );
    assert_eq!(
        access.get("title"),
        Some(&SurfaceDataVm::String(DOCUMENT_TITLE.to_string()))
    );
    assert_eq!(
        access.get("placeholder"),
        Some(&SurfaceDataVm::Bool(true))
    );
    assert!(
        !first_message(frame).actions.is_empty(),
        "G02 document cards retain Helix-authored message actions"
    );
    assert!(first_message(frame).actions.iter().all(|action| {
        !action.action_ref.is_empty() && action.surface_epoch == frame.mutations[0].surface_epoch
    }));
}
