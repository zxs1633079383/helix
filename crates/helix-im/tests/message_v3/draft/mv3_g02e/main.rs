// 回归锚: specs/004-messagev3-helix-phase1-send-domain T112/T113，Gate MV3-G02e 草稿域。
//
// 本模块的断言全部指向 `helix_im::draft` 的真实实现：删掉实现或改坏其中任一不变量，
// 这里至少有一条用例会红。参考包（visual 仓 gates/draft/mv3-g02e/）是 wire 形状唯一真源。

use helix_core::effect::{Effect, Row, SqlValue, StorageOp};
use helix_core::port_codec::rows_to_reply_bytes;
use helix_core::tick::AppCommand;
use helix_core::{Module, Tick};
use helix_im::draft::{self, DraftProjection, QueryDraftCommand, SaveDraftCommand};
use serde_json::{json, Value};

#[path = "../../../common.rs"]
mod common;
#[path = "../../support/phase1_reference.rs"]
mod phase1_reference;

use common::{port_reply_err, port_reply_ok, port_reply_with_body, Harness};

const ACCOUNT_ID: &str = "user-actor-444";
const OTHER_ACCOUNT_ID: &str = "user-receiver-445";
const CHANNEL_ID: &str = "channel-phase1-001";
const DRAFT_TEXT: &str = "未发送草稿";
const UPDATED_AT: i64 = 1_700_000_000_000;
const NOW_MS: u64 = 1_700_000_000_500;

// ── fixtures ────────────────────────────────────────────────────────────────

fn harness(account_id: &str) -> Harness {
    let mut harness = Harness::new_with_identity(&[], account_id, "company-phase1", "", "", "");
    // 正例必须先建立真实 channel/team/viewer 归属，才能进入 draft 的渲染闸门。
    harness.seed_channel_scope(CHANNEL_ID, "company-phase1", account_id);
    harness.sink.clear();
    harness.effect_log.clear();
    harness
}

fn save_payload() -> Vec<u8> {
    serde_json::to_vec(&json!({
        "channel_id": CHANNEL_ID,
        "text": DRAFT_TEXT,
        "updated_at": UPDATED_AT,
        "req_id": "mv3-g02e-save-1"
    }))
    .expect("save payload must serialize")
}

fn query_payload() -> Vec<u8> {
    serde_json::to_vec(&json!({
        "channel_id": CHANNEL_ID,
        "req_id": "mv3-g02e-query-1"
    }))
    .expect("query payload must serialize")
}

fn stored_row(account_id: &str, channel_id: &str, text: &str, updated_at: i64) -> Row {
    vec![
        (
            "account_id".to_string(),
            SqlValue::Text(account_id.to_string()),
        ),
        (
            "channel_id".to_string(),
            SqlValue::Text(channel_id.to_string()),
        ),
        ("text".to_string(), SqlValue::Text(text.to_string())),
        ("props".to_string(), SqlValue::Text("{}".to_string())),
        ("updated_at".to_string(), SqlValue::Integer(updated_at)),
    ]
}

/// 本 step 的全部 Emit（本 Gate 的期望恒为 0 条领域事件 / 至多 1 条结构化结果）。
fn emitted(effects: &[Effect]) -> Vec<Value> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Emit { event } => serde_json::from_slice(event.0.as_ref()).ok(),
            _ => None,
        })
        .collect()
}

fn persist_corr(effects: &[Effect]) -> u64 {
    effects
        .iter()
        .find_map(|effect| match effect {
            Effect::Persist { corr, .. } => Some(corr.raw()),
            _ => None,
        })
        .expect("MV3-G02e 必须落一条 Effect::Persist")
}

fn persist_ops(effects: &[Effect]) -> Vec<StorageOp> {
    effects
        .iter()
        .find_map(|effect| match effect {
            Effect::Persist { ops, .. } => Some(ops.clone()),
            _ => None,
        })
        .expect("MV3-G02e 必须落一条 Effect::Persist")
}

fn upsert_row(effects: &[Effect]) -> Row {
    persist_ops(effects)
        .into_iter()
        .find_map(|op| match op {
            StorageOp::BatchUpsert(spec) if spec.table == draft::DRAFT_TABLE => {
                spec.rows.first().cloned()
            }
            _ => None,
        })
        .expect("save 必须写 message_draft 单行")
}

fn text_of(row: &Row, name: &str) -> Option<String> {
    row.iter().find_map(|(column, value)| match value {
        SqlValue::Text(value) if column == name => Some(value.clone()),
        _ => None,
    })
}

fn int_of(row: &Row, name: &str) -> Option<i64> {
    row.iter().find_map(|(column, value)| match value {
        SqlValue::Integer(value) if column == name => Some(*value),
        _ => None,
    })
}

/// `Effect` 不实现 `Clone`/`PartialEq`，跨 step 比较用稳定 Debug 快照。
fn row_snapshot(row: &Row) -> String {
    format!("{row:?}")
}

fn run_save(harness: &mut Harness, payload: &[u8], now_ms: u64) {
    harness.sink.clear();
    draft::handle_save_draft(&mut harness.module, payload, now_ms, &mut harness.sink)
        .expect("save draft must be accepted");
}

fn run_query(harness: &mut Harness, payload: &[u8]) -> QueryDraftCommand {
    harness.sink.clear();
    draft::handle_query_draft(&mut harness.module, payload, &mut harness.sink)
        .expect("query draft must be accepted")
}

fn decode(event: helix_im::event::MessageV3Event) -> Value {
    match event.into_effect() {
        Effect::Emit { event } => {
            serde_json::from_slice(event.0.as_ref()).expect("result must be JSON")
        }
        other => panic!("结果必须是 Emit，实际 {other:?}"),
    }
}

// ── 1. 参考包完整性（REFERENCE 等级）────────────────────────────────────────

/// 证明 MV3-G02e 的 visual 目标参考包完整且未伪装为运行时证据。
#[test]
fn reference_package_is_complete() {
    phase1_reference::assert_reference_gate("MV3-G02e", "draft/mv3-g02e");
}

// ── 2. inbound sample 可反序列化为预期 Tick ─────────────────────────────────

/// 参考包 `inbound-sample.json` 的两条 `Tick::Command` 必须能被真实实现解析。
#[test]
fn inbound_sample_deserializes_into_expected_commands() {
    let gate_root = phase1_reference::reference_root().join("draft/mv3-g02e");
    let sample = phase1_reference::read_json(&gate_root.join("inbound-sample.json"));
    let inputs = sample["rust_entry_inputs"]
        .as_array()
        .expect("inbound sample must declare rust_entry_inputs");
    assert_eq!(inputs.len(), 2, "MV3-G02e 由 save + query 两条命令组成");

    let save = &inputs[0]["value"];
    assert_eq!(save["name"], "im_save_draft");
    let save_payload = serde_json::to_vec(&save["payload"]).expect("payload must serialize");
    let parsed = SaveDraftCommand::parse(&save_payload, ACCOUNT_ID, NOW_MS)
        .expect("reference save payload must parse");
    assert!(
        save["payload"].get("updated_at").is_none(),
        "updated_at 归 Host Clock，Angular/Rust sample 不得提交"
    );
    assert_eq!(
        parsed.draft,
        DraftProjection {
            account_id: ACCOUNT_ID.to_string(),
            channel_id: CHANNEL_ID.to_string(),
            text: DRAFT_TEXT.to_string(),
            props: serde_json::Map::new(),
            updated_at: NOW_MS as i64,
        }
    );

    let query = &inputs[1]["value"];
    assert_eq!(query["name"], "im_query_draft");
    let query_payload = serde_json::to_vec(&query["payload"]).expect("payload must serialize");
    let parsed = QueryDraftCommand::parse(&query_payload, ACCOUNT_ID)
        .expect("reference query payload must parse");
    assert_eq!(parsed.account_id, ACCOUNT_ID);
    assert_eq!(parsed.channel_id, CHANNEL_ID);
}

// ── 3. 首次 Effect 顺序：Persist only，零事件 ───────────────────────────────

/// `effects-contract.json.allowed_effect_tags = ["Persist"]`：首个 Effect 集合恰为一条 Persist。
#[test]
fn save_first_effect_set_is_exactly_one_persist_and_zero_events() {
    let mut harness = harness(ACCOUNT_ID);
    run_save(&mut harness, &save_payload(), NOW_MS);
    let effects = harness.sink.as_slice();

    assert_eq!(
        effects.len(),
        1,
        "MV3-G02e 首个 Effect 集合只能有一条 Persist"
    );
    assert!(matches!(effects[0], Effect::Persist { .. }));
    assert!(
        emitted(effects).is_empty(),
        "MV3-G02e 零事件：Persist 阶段不得 Emit"
    );
    assert!(!effects.iter().any(|effect| matches!(
        effect,
        Effect::Http { .. }
            | Effect::HttpFire { .. }
            | Effect::UploadFile { .. }
            | Effect::ScheduleTimer { .. }
    )));

    let row = upsert_row(effects);
    assert_eq!(text_of(&row, "account_id").as_deref(), Some(ACCOUNT_ID));
    assert_eq!(text_of(&row, "channel_id").as_deref(), Some(CHANNEL_ID));
    assert_eq!(text_of(&row, "text").as_deref(), Some(DRAFT_TEXT));
    assert_eq!(int_of(&row, "updated_at"), Some(UPDATED_AT));
}

// ── 4. continuation 驱动最终结果 ────────────────────────────────────────────

/// PersistOk 才释放唯一结构化 Command Result，形状与 `outbound-sample.json` 一致。
#[test]
fn persist_ok_releases_the_single_structured_draft_result() {
    let mut harness = harness(ACCOUNT_ID);
    run_save(&mut harness, &save_payload(), NOW_MS);
    let corr = persist_corr(harness.sink.as_slice());

    harness.step(port_reply_ok(corr), NOW_MS + 1);
    let events = emitted(harness.sink.as_slice());
    assert_eq!(events.len(), 1, "PersistOk 只释放一条 Command Result");
    assert_eq!(
        events[0],
        json!({
            "event": draft::DRAFT_RESULT_EVENT,
            "data": {
                "req_id": "mv3-g02e-save-1",
                "body": {
                    "ok": true,
                    "draft": {
                        "accountId": ACCOUNT_ID,
                        "channelId": CHANNEL_ID,
                        "text": DRAFT_TEXT,
                        "props": {},
                        "updatedAt": UPDATED_AT
                    }
                }
            }
        })
    );
    // 结果通道不是 21 个 ImGateEvent kind 之一：Adapter 不会把它当业务事件。
    let name = events[0]["event"].as_str().unwrap_or_default();
    assert!(!name.starts_with("im:post:"));
    assert!(!name.starts_with("im:channel:"));
}

// ── 5. 保存失败保旧草稿 ─────────────────────────────────────────────────────

/// PersistErr → 零结果、零事件；旧草稿因事务未提交而原样保留。
#[test]
fn persist_failure_preserves_previous_draft_and_emits_nothing() {
    let mut harness = harness(ACCOUNT_ID);
    run_save(&mut harness, &save_payload(), NOW_MS);
    let first_corr = persist_corr(harness.sink.as_slice());
    harness.step(port_reply_ok(first_corr), NOW_MS + 1);
    assert_eq!(emitted(harness.sink.as_slice()).len(), 1);

    let overwrite = serde_json::to_vec(&json!({
        "channel_id": CHANNEL_ID,
        "text": "会失败的新草稿",
        "req_id": "mv3-g02e-save-2"
    }))
    .expect("payload must serialize");
    run_save(&mut harness, &overwrite, NOW_MS + 2);
    let second_corr = persist_corr(harness.sink.as_slice());
    harness.step(port_reply_err(second_corr), NOW_MS + 3);

    assert!(
        harness.sink.as_slice().is_empty(),
        "保存失败必须零 Effect：不得回灌结果，也不得补写空草稿"
    );

    // 失败没有改动 durable 行 → 读回仍是第一份草稿。
    let command = run_query(&mut harness, &query_payload());
    let event = command
        .result_event(&[stored_row(ACCOUNT_ID, CHANNEL_ID, DRAFT_TEXT, UPDATED_AT)])
        .expect("query result must encode");
    assert_eq!(decode(event)["data"]["body"]["draft"]["text"], DRAFT_TEXT);
}

// ── 6. last-write-wins ──────────────────────────────────────────────────────

/// 同一 `(accountId, channelId)` 重复保存收敛为单行覆盖写（`INSERT OR REPLACE`），不追加历史。
#[test]
fn same_account_channel_key_is_last_write_wins() {
    /// 逐条校验「单 op、单行、INSERT OR REPLACE」的写形状。
    fn assert_last_write_wins_shape(harness: &Harness) {
        let ops = persist_ops(harness.sink.as_slice());
        assert_eq!(ops.len(), 1, "草稿写恒为单 op，O(1) 无 read-modify-write");
        match &ops[0] {
            StorageOp::BatchUpsert(spec) => {
                assert_eq!(spec.table, draft::DRAFT_TABLE);
                assert_eq!(spec.rows.len(), 1);
                // conflict_key=None → INSERT OR REPLACE，直接落在复合主键
                // PRIMARY KEY (account_id, channel_id) 上，等价 last-write-wins。
                assert_eq!(spec.conflict_key, None);
                assert!(spec.exclude_from_update.is_empty());
            }
            other => panic!("草稿写必须是 BatchUpsert，实际 {other:?}"),
        }
    }

    let mut harness = harness(ACCOUNT_ID);
    run_save(&mut harness, &save_payload(), NOW_MS);
    assert_last_write_wins_shape(&harness);
    let first_row = upsert_row(harness.sink.as_slice());

    let second_payload = serde_json::to_vec(&json!({
        "channel_id": CHANNEL_ID,
        "text": "第二版草稿",
        "updated_at": UPDATED_AT + 1_000
    }))
    .expect("payload must serialize");
    run_save(&mut harness, &second_payload, NOW_MS + 5);
    assert_last_write_wins_shape(&harness);
    let latest = upsert_row(harness.sink.as_slice());

    assert_eq!(
        text_of(&first_row, "channel_id"),
        text_of(&latest, "channel_id"),
        "两次写落在同一主键上才谈得上 last-write-wins"
    );
    assert_eq!(text_of(&latest, "text").as_deref(), Some("第二版草稿"));
    assert_eq!(int_of(&latest, "updated_at"), Some(UPDATED_AT + 1_000));
}

// ── 7. 账号隔离 ────────────────────────────────────────────────────────────

/// 账号身份来自运行时，不来自 payload；查询按 `account_id` 作用域，跨账号行一律当 miss。
#[test]
fn account_isolation_scopes_both_write_and_read() {
    let payload = save_payload();
    let mut mine = harness(ACCOUNT_ID);
    let mut theirs = harness(OTHER_ACCOUNT_ID);
    run_save(&mut mine, &payload, NOW_MS);
    let my_row = upsert_row(mine.sink.as_slice());
    run_save(&mut theirs, &payload, NOW_MS);
    let their_row = upsert_row(theirs.sink.as_slice());
    assert_eq!(text_of(&my_row, "account_id").as_deref(), Some(ACCOUNT_ID));
    assert_eq!(
        text_of(&their_row, "account_id").as_deref(),
        Some(OTHER_ACCOUNT_ID)
    );

    let command = run_query(&mut mine, &query_payload());
    match &persist_ops(mine.sink.as_slice())[0] {
        StorageOp::ScopedGet(spec) => {
            assert_eq!(spec.table, draft::DRAFT_TABLE);
            assert_eq!(spec.scope_col, "account_id");
            assert!(matches!(&spec.scope_val, SqlValue::Text(value) if value == ACCOUNT_ID));
            assert_eq!(spec.key_col, "channel_id");
            assert!(matches!(&spec.key_val, SqlValue::Text(value) if value == CHANNEL_ID));
        }
        other => panic!("查询必须走复合作用域 ScopedGet，实际 {other:?}"),
    }

    // 即使 driver 误回了别的账号的行，投影层也必须当 miss，不得泄漏他人草稿。
    let leaked = command
        .result_event(&[stored_row(
            OTHER_ACCOUNT_ID,
            CHANNEL_ID,
            "别人的草稿",
            UPDATED_AT,
        )])
        .expect("query result must encode");
    assert_eq!(decode(leaked)["data"]["body"]["draft"], Value::Null);
}

// ── 8. Storage read-back 匹配结果 ───────────────────────────────────────────

/// 查询读回行 → 绝对 `DraftProjection`，字段与 `outbound-sample.json` 逐一对齐。
#[test]
fn query_readback_row_projects_the_absolute_draft() {
    let mut harness = harness(ACCOUNT_ID);
    let command = run_query(&mut harness, &query_payload());
    assert_eq!(harness.sink.as_slice().len(), 1);
    assert!(matches!(harness.sink.as_slice()[0], Effect::Persist { .. }));
    assert!(
        emitted(harness.sink.as_slice()).is_empty(),
        "查询自身不得 Emit 任何事件"
    );

    let event = command
        .result_event(&[stored_row(ACCOUNT_ID, CHANNEL_ID, DRAFT_TEXT, UPDATED_AT)])
        .expect("query result must encode");
    assert_eq!(
        decode(event),
        json!({
            "event": draft::DRAFT_RESULT_EVENT,
            "data": {
                "req_id": "mv3-g02e-query-1",
                "body": {
                    "ok": true,
                    "draft": {
                        "accountId": ACCOUNT_ID,
                        "channelId": CHANNEL_ID,
                        "text": DRAFT_TEXT,
                        "props": {},
                        "updatedAt": UPDATED_AT
                    }
                }
            }
        })
    );
}

/// 无草稿是**合法成功结果**（`draft: null`），不是错误——切会话链路不得被读取失败拖垮。
#[test]
fn query_miss_returns_null_draft_instead_of_error() {
    let command =
        QueryDraftCommand::parse(&query_payload(), ACCOUNT_ID).expect("query payload must parse");
    let payload = decode(command.result_event(&[]).expect("miss result must encode"));
    assert_eq!(payload["data"]["body"]["ok"], true);
    assert_eq!(payload["data"]["body"]["draft"], Value::Null);
}

// ── 9. 重复 Command 幂等 ────────────────────────────────────────────────────

/// 同一 Command 重放产出同一份写集合与同一份结果字节（Angular 侧状态不变）。
#[test]
fn duplicate_command_replay_is_idempotent() {
    let payload = save_payload();
    let mut harness = harness(ACCOUNT_ID);
    run_save(&mut harness, &payload, NOW_MS);
    let first_corr = persist_corr(harness.sink.as_slice());
    let first_row = row_snapshot(&upsert_row(harness.sink.as_slice()));
    harness.step(port_reply_ok(first_corr), NOW_MS + 1);
    let first_event = emitted(harness.sink.as_slice());

    run_save(&mut harness, &payload, NOW_MS);
    let second_corr = persist_corr(harness.sink.as_slice());
    let second_row = row_snapshot(&upsert_row(harness.sink.as_slice()));
    assert_ne!(first_corr, second_corr, "每次写必须有独立 correlation");
    harness.step(port_reply_ok(second_corr), NOW_MS + 2);
    let second_event = emitted(harness.sink.as_slice());

    assert_eq!(first_row, second_row);
    assert_eq!(first_event, second_event);
}

// ── 10. 错误与越权输入 fail-closed ──────────────────────────────────────────

/// 未知字段、越权 `account_id`、坏类型与缺身份一律 fail-closed，且不产生任何 Effect。
#[test]
fn unknown_and_privileged_inputs_fail_closed() {
    let cases: Vec<(&str, Value)> = vec![
        (
            "未知字段",
            json!({"channel_id": CHANNEL_ID, "readBits": "11"}),
        ),
        (
            "越权注入账号",
            json!({"channel_id": CHANNEL_ID, "account_id": OTHER_ACCOUNT_ID}),
        ),
        ("缺 channel_id", json!({"text": DRAFT_TEXT})),
        ("空 channel_id", json!({"channel_id": ""})),
        (
            "props 非对象",
            json!({"channel_id": CHANNEL_ID, "props": ["not-an-object"]}),
        ),
        (
            "text 非字符串",
            json!({"channel_id": CHANNEL_ID, "text": 42}),
        ),
        (
            "updated_at 非正整数",
            json!({"channel_id": CHANNEL_ID, "updated_at": 0}),
        ),
    ];
    for (name, payload) in cases {
        let bytes = serde_json::to_vec(&payload).expect("payload must serialize");
        assert!(
            SaveDraftCommand::parse(&bytes, ACCOUNT_ID, NOW_MS).is_err(),
            "{name} 必须 fail-closed"
        );

        let mut harness = harness(ACCOUNT_ID);
        harness.sink.clear();
        assert!(
            draft::handle_save_draft(&mut harness.module, &bytes, NOW_MS, &mut harness.sink)
                .is_err(),
            "{name} 必须在模块入口被拒"
        );
        assert!(
            harness.sink.as_slice().is_empty(),
            "{name} 被拒后不得留下任何 Effect"
        );
    }

    // payload 合法但运行时无身份 → 仍 fail-closed（草稿绝不写进匿名账号）。
    assert!(SaveDraftCommand::parse(&save_payload(), "", NOW_MS).is_err());
    assert!(QueryDraftCommand::parse(&query_payload(), "").is_err());
    // 非对象 / 非 JSON payload。
    assert!(SaveDraftCommand::parse(b"[]", ACCOUNT_ID, NOW_MS).is_err());
    assert!(QueryDraftCommand::parse(b"not-json", ACCOUNT_ID).is_err());
}

// ── 11. PC / Mobile 输出 byte-exact ─────────────────────────────────────────

/// 同一身份、同一入参在两个独立 `ImModule` 实例上产出 byte-exact 的结果字节。
#[test]
fn pc_and_mobile_results_are_byte_exact() {
    use helix_core::tick::{PortOutcome, ReplyBytes};
    use helix_core::{Correlation, Module, Tick};

    let payload = save_payload();
    let bytes = |account: &str| -> Vec<u8> {
        let fixture = harness(account);
        let mut module = fixture.module;
        let mut sink = fixture.sink;
        draft::handle_save_draft(&mut module, &payload, NOW_MS, &mut sink)
            .expect("save must be accepted");
        let corr = persist_corr(sink.as_slice());
        sink.clear();
        module
            .handle(
                &Tick::PortReply {
                    corr: Correlation::from_raw(corr),
                    outcome: PortOutcome::Ok(ReplyBytes::default()),
                },
                NOW_MS + 1,
                &mut sink,
            )
            .expect("persist ok must be handled");
        sink.as_slice()
            .iter()
            .find_map(|effect| match effect {
                Effect::Emit { event } => Some(event.0.as_ref().to_vec()),
                _ => None,
            })
            .expect("PersistOk 必须释放结果")
    };
    assert_eq!(bytes(ACCOUNT_ID), bytes(ACCOUNT_ID));
    assert_ne!(
        bytes(ACCOUNT_ID),
        bytes(OTHER_ACCOUNT_ID),
        "账号不同必须产出不同的绝对投影"
    );
}

// ── 12. 草稿是 persona-local，不进领域事件面 ────────────────────────────────

/// 全链路（save + PersistOk + query）零 `im:post:*` / `im:channel:*` 事件。
#[test]
fn draft_domain_never_publishes_a_message_domain_event() {
    let mut harness = harness(ACCOUNT_ID);
    let mut all = Vec::new();
    run_save(&mut harness, &save_payload(), NOW_MS);
    all.extend(emitted(harness.sink.as_slice()));
    let corr = persist_corr(harness.sink.as_slice());
    harness.step(port_reply_ok(corr), NOW_MS + 1);
    all.extend(emitted(harness.sink.as_slice()));
    let _ = run_query(&mut harness, &query_payload());
    all.extend(emitted(harness.sink.as_slice()));

    for event in &all {
        let name = event["event"].as_str().unwrap_or_default();
        assert_eq!(
            name,
            draft::DRAFT_RESULT_EVENT,
            "MV3-G02e 只允许结构化结果通道，出现了 {name}"
        );
    }
    assert_eq!(all.len(), 1, "全链路恰好一条结构化 Command Result");
}

// ── 13. 真实链路可达性（T113 核心：草稿域不再是死代码）─────────────────────

/// `Tick::Command` 构造器：走**公开命令名**，不走内部 handler 直调。
fn draft_tick(name: &'static str, payload: Vec<u8>) -> Tick {
    Tick::Command(AppCommand::new(name, payload))
}

/// 本 step 的全部 Effect 数量 + 首个 Effect 是否 Persist。
fn only_persist(effects: &[Effect]) -> &StorageOp {
    assert_eq!(effects.len(), 1, "草稿命令恒为单 Effect");
    match &effects[0] {
        Effect::Persist { ops, .. } => {
            assert_eq!(ops.len(), 1, "草稿命令恒为单 StorageOp");
            &ops[0]
        }
        other => panic!("草稿命令必须落 Effect::Persist，实际 {other:?}"),
    }
}

/// `Module::accepts` 必须放行草稿族——accepts 闸没放行 = 整族命令在 Runtime 被静默丢弃。
///
/// 同时钉死 `is_draft_command` 是闭集：非草稿命令不得被草稿域抢走。
#[test]
fn accepts_gate_admits_the_draft_command_family() {
    let module = harness(ACCOUNT_ID).module;
    for name in ["im_save_draft", "im_query_draft"] {
        assert!(draft::is_draft_command(name), "{name} 必须在草稿命令闭集内");
    }
    assert!(module.accepts(&draft_tick("im_save_draft", save_payload())));
    assert!(module.accepts(&draft_tick("im_query_draft", query_payload())));

    // 闭集边界：草稿域不得越权认领发送 / 读族命令。
    assert!(!draft::is_draft_command("im_send_message"));
    assert!(!draft::is_draft_command("im_get_posts"));
    assert!(!draft::is_draft_command("im_save_drafts"));
    assert!(!draft::is_draft_command(""));
    assert_eq!(draft::DRAFT_COMMANDS.len(), 2);
}

/// **端到端可达性**：`Tick::Command im_save_draft` → Persist → PersistOk → 唯一结构化结果。
///
/// 这条用例只经 `Module::handle` 公开入口，不直调 `draft::handle_save_draft`：
/// 只要 `trait_impl.rs` 的 Command 分发臂缺失（命令落回 `_` unhandled 分支），
/// 首个 step 就会是零 Effect 而红。
#[test]
fn save_draft_is_reachable_end_to_end_through_tick_command() {
    let mut harness = harness(ACCOUNT_ID);
    harness.step(draft_tick("im_save_draft", save_payload()), NOW_MS);

    let effects = harness.sink.as_slice();
    assert!(
        !effects.is_empty(),
        "im_save_draft 必须被 Tick::Command 分发臂认领；零 Effect = 命令仍是死代码"
    );
    match only_persist(effects) {
        StorageOp::BatchUpsert(spec) => {
            assert_eq!(spec.table, draft::DRAFT_TABLE);
            assert_eq!(spec.rows.len(), 1);
            assert_eq!(
                text_of(&spec.rows[0], "account_id").as_deref(),
                Some(ACCOUNT_ID),
                "账号身份只能来自运行时 config，不能来自 payload"
            );
            assert_eq!(text_of(&spec.rows[0], "text").as_deref(), Some(DRAFT_TEXT));
        }
        other => panic!("保存必须是 BatchUpsert，实际 {other:?}"),
    }
    assert!(
        emitted(effects).is_empty(),
        "MV3-G02e 零事件：Persist 阶段不得 Emit"
    );

    let corr = persist_corr(effects);
    harness.step(port_reply_ok(corr), NOW_MS + 1);
    let events = emitted(harness.sink.as_slice());
    assert_eq!(
        events.len(),
        1,
        "PersistOk 必须经真实 dispatch 释放唯一 Command Result"
    );
    assert_eq!(events[0]["event"], draft::DRAFT_RESULT_EVENT);
    assert_eq!(events[0]["data"]["req_id"], "mv3-g02e-save-1");
    assert_eq!(
        events[0]["data"]["body"]["draft"],
        json!({
            "accountId": ACCOUNT_ID,
            "channelId": CHANNEL_ID,
            "text": DRAFT_TEXT,
            "props": {},
            "updatedAt": UPDATED_AT
        })
    );
}

/// `Tick::Command im_query_draft` 必须经真实分发臂落一条 `(account_id, channel_id)` 作用域读。
#[test]
fn query_draft_is_reachable_through_tick_command() {
    let mut harness = harness(ACCOUNT_ID);
    harness.step(draft_tick("im_query_draft", query_payload()), NOW_MS);

    let effects = harness.sink.as_slice();
    assert!(
        !effects.is_empty(),
        "im_query_draft 必须被 Tick::Command 分发臂认领"
    );
    match only_persist(effects) {
        StorageOp::ScopedGet(spec) => {
            assert_eq!(spec.table, draft::DRAFT_TABLE);
            assert_eq!(spec.scope_col, "account_id");
            assert!(matches!(&spec.scope_val, SqlValue::Text(v) if v == ACCOUNT_ID));
            assert_eq!(spec.key_col, "channel_id");
            assert!(matches!(&spec.key_val, SqlValue::Text(v) if v == CHANNEL_ID));
        }
        other => panic!("查询必须是 ScopedGet，实际 {other:?}"),
    }
    assert!(emitted(effects).is_empty(), "查询自身不得 Emit 任何事件");
}

/// 真实 Command -> ScopedGet -> PortReply 链必须回灌账号隔离后的绝对草稿。
#[test]
fn query_draft_readback_returns_structured_result_through_real_dispatch() {
    let mut harness = harness(ACCOUNT_ID);
    harness.step(draft_tick("im_query_draft", query_payload()), NOW_MS);
    let corr = persist_corr(harness.sink.as_slice());
    let row = stored_row(ACCOUNT_ID, CHANNEL_ID, DRAFT_TEXT, UPDATED_AT);
    harness.step(
        port_reply_with_body(corr, rows_to_reply_bytes(&[row])),
        NOW_MS + 1,
    );
    let events = emitted(harness.sink.as_slice());
    assert_eq!(
        events.len(),
        1,
        "query readback must emit one command result"
    );
    assert_eq!(events[0]["event"], draft::DRAFT_RESULT_EVENT);
    assert_eq!(events[0]["data"]["req_id"], "mv3-g02e-query-1");
    assert_eq!(events[0]["data"]["body"]["ok"], true);
    assert_eq!(
        events[0]["data"]["body"]["draft"],
        json!({
            "accountId": ACCOUNT_ID,
            "channelId": CHANNEL_ID,
            "text": DRAFT_TEXT,
            "props": {},
            "updatedAt": UPDATED_AT
        })
    );
}

/// 真实入口的 fail-closed：越权 / 未知字段 payload 经 `Module::handle` 必须报错且零 Effect。
///
/// 直调 handler 的第 10 组用例证明**函数**会拒；本用例证明**真实分发臂**也会拒——
/// 分发臂若吞掉 `ImError`（`let _ = ...`），这里会红。
#[test]
fn tick_command_entry_is_fail_closed_with_zero_effects() {
    let cases: Vec<(&str, &'static str, Value)> = vec![
        (
            "save 越权注入账号",
            "im_save_draft",
            json!({"channel_id": CHANNEL_ID, "account_id": OTHER_ACCOUNT_ID}),
        ),
        (
            "save 未知字段",
            "im_save_draft",
            json!({"channel_id": CHANNEL_ID, "readBits": "11"}),
        ),
        ("save 缺 channel_id", "im_save_draft", json!({"text": "x"})),
        (
            "query 未知字段",
            "im_query_draft",
            json!({"channel_id": CHANNEL_ID, "account_id": OTHER_ACCOUNT_ID}),
        ),
        (
            "query 空 channel_id",
            "im_query_draft",
            json!({"channel_id": ""}),
        ),
    ];
    for (case, name, payload) in cases {
        let mut harness = harness(ACCOUNT_ID);
        harness.sink.clear();
        let bytes = serde_json::to_vec(&payload).expect("payload must serialize");
        let result = harness
            .module
            .handle(&draft_tick(name, bytes), NOW_MS, &mut harness.sink);
        assert!(result.is_err(), "{case} 必须在真实分发臂上 fail-closed");
        assert!(
            harness.sink.as_slice().is_empty(),
            "{case} 被拒后不得留下任何 Effect"
        );
    }
}

/// 运行时身份是写入草稿账号的**唯一**来源：同一 payload 在两个身份下落到两行不同主键。
///
/// 走真实 `Tick::Command` 入口，覆盖「分发臂错传了 payload 里的 account 或写死空账号」。
#[test]
fn tick_command_binds_draft_to_the_runtime_identity() {
    let read_account = |account: &str| -> String {
        let mut harness = harness(account);
        harness.step(draft_tick("im_save_draft", save_payload()), NOW_MS);
        text_of(&upsert_row(harness.sink.as_slice()), "account_id")
            .expect("写入行必须带 account_id")
    };
    assert_eq!(read_account(ACCOUNT_ID), ACCOUNT_ID);
    assert_eq!(read_account(OTHER_ACCOUNT_ID), OTHER_ACCOUNT_ID);

    // 无运行时身份 → 真实入口拒绝，草稿绝不写进匿名账号。
    let mut anonymous = harness("");
    anonymous.sink.clear();
    assert!(anonymous
        .module
        .handle(
            &draft_tick("im_save_draft", save_payload()),
            NOW_MS,
            &mut anonymous.sink
        )
        .is_err());
    assert!(anonymous.sink.as_slice().is_empty());
}
