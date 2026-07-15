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
    projections: Vec<Projection>,
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
    schema: String,
}

fn matrix() -> Matrix {
    serde_json::from_str(include_str!("../contracts/strict-matrix.v1.json"))
        .expect("strict matrix JSON must deserialize")
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
                projection.schema, "RenderNode/v1",
                "{} must be render-ready",
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
