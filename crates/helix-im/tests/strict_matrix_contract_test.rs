use std::collections::HashSet;

use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct Matrix {
    schema_version: String,
    contract_epoch: u32,
    coverage: Coverage,
    forbidden_aliases: Vec<String>,
    case_defaults: CaseDefaults,
    uc_cases: Vec<UcCase>,
}

#[derive(Debug, Deserialize)]
struct Coverage {
    minimum_gate_count: usize,
    required_gate_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct CaseDefaults {
    owner: Owner,
    evidence: Evidence,
}

#[derive(Debug, Deserialize)]
struct Owner {
    business_state: String,
    host: String,
    ui: String,
}

#[derive(Debug, Deserialize)]
struct Evidence {
    required_facets: Vec<String>,
    correlation_keys: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct UcCase {
    gate_id: String,
    lifecycle: String,
    contract_state: String,
    commands: Vec<CommandStep>,
    http: HttpContract,
    ws: WsContract,
    remote_store: RemoteStore,
    persistence: Vec<String>,
    projections: Vec<Projection>,
    negative_assertions: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct HttpContract {
    request: String,
    response: String,
}

#[derive(Debug, Deserialize)]
struct WsContract {
    events: String,
}

#[derive(Debug, Deserialize)]
struct RemoteStore {
    database: String,
    tables: Vec<String>,
    contract: String,
}

#[derive(Debug, Deserialize)]
struct CommandStep {
    name: String,
    input: StrictInput,
}

#[derive(Debug, Deserialize)]
struct StrictInput {
    required: Vec<String>,
    optional: Vec<String>,
    forbid_unknown: bool,
}

#[derive(Debug, Deserialize)]
struct Projection {
    event: String,
    schema: String,
    #[serde(default)]
    required_data_fields: Vec<String>,
    #[serde(default)]
    merge_contract: String,
}

#[test]
fn g12_freezes_reply_projection_ordering_and_paging_contract() {
    let matrix = matrix();
    let g12 = matrix
        .uc_cases
        .iter()
        .find(|entry| entry.gate_id == "G-12")
        .expect("strict matrix must contain G-12");
    let projection = g12
        .projections
        .iter()
        .find(|projection| {
            projection.event == "im:interaction:frame"
                && projection.schema == "InteractionFrame/v2"
        })
        .expect("G-12 must declare its InteractionFrame/v2 projection");
    assert_eq!(
        projection.required_data_fields,
        [
            "reqId",
            "channelId",
            "rootMessageId",
            "projectionMode",
            "revision",
            "replyIds",
            "nodes",
            "replyCount",
            "cursor",
            "hasMore",
        ]
    );
    assert!(projection.merge_contract.contains("旧 revision"));
    assert!(projection.merge_contract.contains("append"));
}

#[test]
fn reply_and_quick_reply_commands_freeze_the_canonical_actor_boundary() {
    let matrix = matrix();
    for gate_id in ["G-01", "G-02"] {
        let send = gate(&matrix, gate_id)
            .commands
            .iter()
            .find(|command| command.name == "im_send_message")
            .unwrap_or_else(|| panic!("{gate_id} must declare im_send_message"));
        assert!(send.input.optional.contains(&"replied".to_string()));
        assert!(send.input.optional.contains(&"req_id".to_string()));
        assert!(!send.input.optional.contains(&"reply_to_id".to_string()));
    }

    let quick_reply = gate(&matrix, "G-06")
        .commands
        .iter()
        .find(|command| command.name == "im_quick_reply")
        .expect("G-06 must declare im_quick_reply");
    assert_eq!(
        quick_reply.input.required,
        vec!["post_id".to_string(), "emoji".to_string()]
    );
    assert_eq!(quick_reply.input.optional, vec!["req_id".to_string()]);
    assert!(!quick_reply.input.required.contains(&"user_id".to_string()));
    assert!(gate(&matrix, "G-06")
        .negative_assertions
        .iter()
        .any(|rule| rule.contains("鉴权 session") && rule.contains("user_id")));
}

fn matrix() -> Matrix {
    serde_json::from_str(include_str!("../contracts/strict-matrix.v1.json"))
        .expect("strict matrix JSON must deserialize")
}

fn gate<'a>(matrix: &'a Matrix, gate_id: &str) -> &'a UcCase {
    matrix
        .uc_cases
        .iter()
        .find(|entry| entry.gate_id == gate_id)
        .unwrap_or_else(|| panic!("strict matrix must contain {gate_id}"))
}

#[test]
fn gs_sync_contracts_freeze_go_main_abi_without_collapsing_evidence_planes() {
    let matrix = matrix();
    let gs1 = gate(&matrix, "G-S1");
    assert!(!gs1.http.response.contains("BLOCKED"));
    for fragment in [
        "CommonRes.status=SUCCESS",
        "data=null",
        "异步 worker 已受理",
        "不表示同步完成",
    ] {
        assert!(
            gs1.http.response.contains(fragment),
            "G-S1 HTTP response must freeze {fragment}"
        );
    }
    for fragment in [
        "increment_channel.data",
        "lastEventSeq",
        "needSync",
        "increment_channel_end",
    ] {
        assert!(
            gs1.ws.events.contains(fragment),
            "G-S1 WS contract must freeze {fragment}"
        );
    }
    assert_eq!(
        gs1.persistence,
        ["channel", "channel_member", "channel_event_cursor"]
    );
    assert!(gs1
        .negative_assertions
        .iter()
        .any(|rule| rule.contains("本地持久化") && rule.contains("im:channels:loaded")));

    for gate_id in ["G-S2", "G-S3"] {
        let gate = gate(&matrix, gate_id);
        assert!(!gate.http.response.contains("BLOCKED"));
        for fragment in [
            "CommonRes.status=SUCCESS",
            "entries",
            "serverTime",
            "no_change",
            "events",
            "too_long",
            "nextSeq",
            "resetTo",
            "messages",
            "500",
            "5000",
        ] {
            assert!(
                gate.http.response.contains(fragment),
                "{gate_id} HTTP response must freeze {fragment}"
            );
        }
        assert!(gate.ws.events.contains("不产生业务 WS"));
        assert_eq!(gate.persistence, ["channel_event_cursor", "message"]);
        assert!(gate
            .negative_assertions
            .iter()
            .any(|rule| rule.contains("HTTP SUCCESS") && rule.contains("本地事务")));
    }

    let gs3 = gate(&matrix, "G-S3");
    assert!(gs3.ws.events.contains("gaps"));
    assert!(gs3.ws.events.contains("hashMismatch"));
    assert!(gs3.ws.events.contains("普通 pong"));
    assert!(gs3.http.response.contains("trigger=pong_gap"));
    assert!(gs3.http.response.contains("普通同步"));
    for gate_id in ["G-S1", "G-S2", "G-S3", "G-S4"] {
        let gate = gate(&matrix, gate_id);
        assert!(
            gate.http
                .response
                .contains("crates/helix-im/tests/gs_persist_barrier_test.rs"),
            "{gate_id} 必须引用可执行持久化屏障证据"
        );
        assert!(gate
            .negative_assertions
            .iter()
            .any(|rule| rule.contains("Persist") || rule.contains("持久化")));
    }
}

#[test]
fn capabilities_publish_only_canonical_send_and_actual_failure_events() {
    let matrix = matrix();
    let forbidden: HashSet<_> = matrix
        .forbidden_aliases
        .iter()
        .map(String::as_str)
        .collect();
    let caps: serde_json::Value =
        serde_json::from_slice(&helix_im::client_api::capabilities_json()).unwrap();
    let commands = caps["commands"].as_array().unwrap();
    let events = caps["events"].as_array().unwrap();

    assert!(commands.iter().any(|name| name == "im_send_message"));
    assert!(!commands.iter().any(|name| name == "im_send"));
    for alias in forbidden {
        assert!(
            !commands.iter().any(|name| name == alias),
            "capabilities leaked forbidden alias {alias}"
        );
    }
    assert!(events.iter().any(|event| event == "im:post:send-failed"));
    assert!(events
        .iter()
        .any(|event| event == "im:post:increment-failed"));
    assert!(events.iter().any(|event| event == "im:sync:state"));
    assert_eq!(caps["features"]["helixOwnedTemporaryId"], true);

    let send = gate(&matrix, "G-03")
        .commands
        .iter()
        .find(|command| command.name == "im_send_message")
        .expect("G-03 must freeze canonical send");
    assert!(!send
        .input
        .required
        .iter()
        .any(|field| field == "temporary_id"));
    assert!(!send
        .input
        .optional
        .iter()
        .any(|field| field == "temporary_id"));
    assert!(gate(&matrix, "G-01")
        .negative_assertions
        .iter()
        .any(|rule| rule.contains("不得生成 temporary_id")));
}

fn is_direct_core_command(name: &str) -> bool {
    matches!(
        name,
        "im_send_message"
            | "im_retry_send"
            | "im_retry_upload"
            | "im_reconnect"
            | "im_relay_messages"
    ) || helix_im::outbound::is_outbound(name)
        || helix_im::query::is_query(name)
}

#[test]
fn g03_freezes_media_pipeline_ownership_barriers_and_real_readback() {
    let matrix = matrix();
    let g03 = gate(&matrix, "G-03");
    let send = g03
        .commands
        .iter()
        .find(|command| command.name == "im_send_message")
        .expect("G-03 must freeze canonical send");

    assert_eq!(g03.lifecycle, "active");
    assert_eq!(g03.contract_state, "declared");
    assert_eq!(send.input.required, ["channel_id", "type", "props"]);
    assert_eq!(send.input.optional, ["text"]);
    assert!(send.input.forbid_unknown);
    assert!(g03.persistence.iter().any(|item| item == "pending_media"));

    for token in [
        "props.file.mediaInput",
        "props.rich[].mediaInput",
        "localPath",
        "fileName",
        "contentType",
        "size",
        "sha256",
        "/media/upload/prepare",
        "PUT",
        "/media/upload/complete",
        "pending_media",
        "posts/create",
    ] {
        assert!(
            g03.http.request.contains(token),
            "G-03 request contract must contain {token}"
        );
    }
    for token in [
        "completed",
        "objectKey",
        "size",
        "contentType",
        "sha256",
        "/media/get",
        "GET",
        "Content-Type",
    ] {
        assert!(
            g03.http.response.contains(token),
            "G-03 response/evidence contract must contain {token}"
        );
    }

    for assertion in [
        "Mobile 不得提供 Go-owned 字段",
        "complete 前不得调用 posts/create",
        "mismatch/corrupt/restart record 必须 fail-closed",
        "真实链必须以 /media/get 与 GET object 对拍 size/contentType/SHA",
    ] {
        assert!(
            g03.negative_assertions.iter().any(|rule| rule == assertion),
            "G-03 must freeze negative assertion: {assertion}"
        );
    }
}

// 冻结建群、创建话题与群设置的权威 HTTP 字段边界。
#[test]
fn uc15_uc16_freeze_canonical_go_request_and_authoritative_response() {
    let matrix = matrix();
    assert_eq!(matrix.contract_epoch, 11);

    let create = gate(&matrix, "G-15a");
    assert!(create
        .http
        .request
        .contains("{type,userIds,displayName,forceCreate}"));
    for invariant in [
        "pictureType=USER",
        "picture.userIds",
        "不得提交 team/role/owner/picture",
    ] {
        assert!(
            create.http.request.contains(invariant),
            "G-15a request 必须冻结默认头像边界: {invariant}"
        );
    }
    for field in [
        "data",
        "id",
        "displayName",
        "type",
        "teamId",
        "pictureType",
        "picture",
    ] {
        assert!(
            create.http.response.contains(field),
            "G-15a response 缺 {field}"
        );
    }

    let topic = gate(&matrix, "G-15");
    assert!(topic
        .http
        .request
        .contains("{rootId,userIds,displayName?}"));
    for invariant in [
        "display_name",
        "用户确认的话题名",
        "req_id",
        "不能进入 HTTP body",
        "服务端",
        "不得提交空 picture",
    ] {
        assert!(
            topic.http.request.contains(invariant),
            "G-15 request 必须冻结话题默认头像/关联边界: {invariant}"
        );
    }
    for field in ["rootMessageId", "channelId", "revision"] {
        assert!(
            topic.http.response.contains(field),
            "G-15 response 缺 {field}"
        );
    }

    let settings = gate(&matrix, "G-16c");
    assert!(settings.http.request.contains("expectedVersion?"));
    for field in ["channelId", "purpose", "header", "settingVersion"] {
        assert!(
            settings.http.response.contains(field),
            "G-16c response 缺 {field}"
        );
    }
}

#[test]
fn strict_matrix_covers_every_baseline_gate_once_with_helix_owned_evidence() {
    let matrix = matrix();
    assert_eq!(matrix.schema_version, "helix-im-strict-matrix/v1");
    assert!(matrix.contract_epoch >= 1);
    assert!(matrix.uc_cases.len() >= matrix.coverage.minimum_gate_count);
    assert_eq!(matrix.case_defaults.owner.business_state, "helix");
    assert_eq!(matrix.case_defaults.owner.host, "transport-storage-only");
    assert_eq!(matrix.case_defaults.owner.ui, "intent-and-render-only");
    assert_eq!(
        matrix.case_defaults.evidence.required_facets,
        [
            "command",
            "effect",
            "transport",
            "local_store",
            "remote_store",
            "projection",
            "dom",
            "trace"
        ]
    );
    for key in ["run_id", "gate_id", "operation_id", "req_id", "channel_id"] {
        assert!(
            matrix
                .case_defaults
                .evidence
                .correlation_keys
                .contains(&key.to_string()),
            "strict evidence must correlate {key}"
        );
    }

    let seen: HashSet<_> = matrix
        .uc_cases
        .iter()
        .map(|entry| entry.gate_id.as_str())
        .collect();
    assert_eq!(seen.len(), matrix.uc_cases.len(), "Gate must be unique");
    for gate_id in &matrix.coverage.required_gate_ids {
        assert!(
            seen.contains(gate_id.as_str()),
            "missing baseline {gate_id}"
        );
    }
}

#[test]
fn strict_matrix_forbids_aliases_and_requires_exact_snake_case_inputs() {
    let matrix = matrix();
    let forbidden: HashSet<_> = matrix
        .forbidden_aliases
        .iter()
        .map(String::as_str)
        .collect();

    for entry in &matrix.uc_cases {
        assert!(
            !entry.commands.is_empty(),
            "{} missing core command",
            entry.gate_id
        );
        assert!(
            !entry.projections.is_empty(),
            "{} missing projection",
            entry.gate_id
        );
        assert!(
            !entry.http.request.is_empty(),
            "{} missing HTTP request contract",
            entry.gate_id
        );
        assert!(
            !entry.http.response.is_empty(),
            "{} missing HTTP response contract",
            entry.gate_id
        );
        assert!(
            !entry.ws.events.is_empty(),
            "{} missing WS contract",
            entry.gate_id
        );
        assert!(
            matches!(
                entry.remote_store.database.as_str(),
                "mattermost-imv2" | "N/A" | "external-unverified"
            ),
            "{} has unknown remote database {}",
            entry.gate_id,
            entry.remote_store.database
        );
        assert!(
            !entry.remote_store.contract.is_empty(),
            "{} missing remote store contract",
            entry.gate_id
        );
        if entry.remote_store.database == "mattermost-imv2" {
            assert!(
                !entry.remote_store.tables.is_empty(),
                "{} imv2 tables missing",
                entry.gate_id
            );
        }
        for projection in &entry.projections {
            assert_eq!(
                projection.event, "im:interaction:frame",
                "{} must not publish a legacy business projection",
                entry.gate_id
            );
            assert_eq!(
                projection.schema, "InteractionFrame/v2",
                "{} projection schema must be V2-only",
                entry.gate_id
            );
        }
        for command in &entry.commands {
            assert!(
                !forbidden.contains(command.name.as_str()),
                "{} still uses alias {}",
                entry.gate_id,
                command.name
            );
            assert!(
                command.input.forbid_unknown,
                "{} must reject unknown input",
                command.name
            );
            let required: HashSet<_> = command.input.required.iter().collect();
            for field in command
                .input
                .required
                .iter()
                .chain(command.input.optional.iter())
            {
                assert!(
                    field.chars().all(|character| character.is_ascii_lowercase()
                        || character.is_ascii_digit()
                        || character == '_'),
                    "{} has non-snake field {field}",
                    command.name
                );
                assert!(
                    !command.input.optional.contains(field) || !required.contains(field),
                    "{} duplicates required/optional field {field}",
                    command.name
                );
            }
        }
    }
}

#[test]
fn only_implemented_contracts_must_already_exist_in_the_core_dispatch_surface() {
    let matrix = matrix();
    for entry in &matrix.uc_cases {
        assert!(
            !(entry.lifecycle == "active" && entry.contract_state == "blocked"),
            "{} cannot be active and blocked simultaneously",
            entry.gate_id
        );
        if entry.contract_state != "implemented" {
            continue;
        }
        for command in &entry.commands {
            assert!(
                is_direct_core_command(&command.name),
                "{} marks {} implemented but Helix does not dispatch it",
                entry.gate_id,
                command.name
            );
        }
    }
}
