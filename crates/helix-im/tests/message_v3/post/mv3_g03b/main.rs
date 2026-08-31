//! MV3-G03b｜合并转发（canonical `mode: "merge"`）。
//!
//! 与 MV3-G03a 互为数量语义的对照面：无论多少条源消息，merge 模式只产出**单条** `MULTIPLY`，
//! 接收者未读因此只 **+1**（item 模式是 +N）。本文件把两条路径放在同一断言里直接对撞，
//! 任何一侧塌陷成另一侧都会立刻变红。
//!
//! 未读断言不接受测试喂数：本文件内置一个 `channel_member` 迷你驱动，严格按实现写出的
//! `BatchUpsert` / `ScopedGuardedBump`（含 `exclude_from_update` 与前进守卫）演算，
//! 读回再交还实现组装 `im:channel:update`。

use bytes::Bytes;
use helix_core::effect::{Effect, Row, SqlValue, StorageOp};
use helix_core::port_codec::rows_to_reply_bytes;
use helix_core::tick::{InboundBytes, ReplyBytes};
use helix_core::{Correlation, Tick};
use serde_json::{json, Value};
use std::collections::BTreeMap;

#[path = "../../../common.rs"]
mod common;
#[path = "../../support/phase1_reference.rs"]
mod phase1_reference;

use common::{cid, port_reply_ok, port_reply_with_body, sid, Harness};

const TARGET_A: u64 = 5;
const SOURCE_CHANNEL: u64 = 50;
const OTHER_SOURCE_CHANNEL: u64 = 51;
const SOURCE_A: u64 = 61;
const SOURCE_B: u64 = 62;
const SOURCE_C: u64 = 63;
const SENDER: &str = "user-actor-444";
const RECEIVER: &str = "user-receiver-445";
const REQ_ID: &str = "mv3-g03b-forward-1";
const SOURCE_TITLE: &str = "研发群";
const CREATE_AT: i64 = 1_700_000_000_000;

// ─── 参考包 ────────────────────────────────────────────────────────────────

/// 证明 MV3-G03b 的 visual 目标参考包完整且未伪装为运行时证据。
#[test]
fn reference_package_is_complete() {
    phase1_reference::assert_reference_gate("MV3-G03b", "post/mv3-g03b");
}

/// 从参考包读出 canonical 入站载荷与目标出站样本，避免测试凭空发明字段形状。
fn reference_json(file: &str) -> Value {
    let path = phase1_reference::reference_root()
        .join("post/mv3-g03b")
        .join(file);
    phase1_reference::read_json(&path)
}

// ─── channel_member 迷你驱动 ───────────────────────────────────────────────

/// 忠实复演 `BatchUpsert(exclude_from_update)` 与 `ScopedGuardedBump(guard_col>)` 的复合主键表。
#[derive(Default)]
struct MemberTable {
    rows: BTreeMap<(String, String), BTreeMap<String, SqlValue>>,
}

impl MemberTable {
    fn apply(&mut self, ops: &[StorageOp]) {
        for op in ops {
            match op {
                StorageOp::BatchUpsert(spec) if spec.table == "channel_member" => {
                    for row in &spec.rows {
                        let key = (column_text(row, "channel_id"), column_text(row, "user_id"));
                        let entry = self.rows.entry(key).or_default();
                        let inserting = entry.is_empty();
                        for (column, value) in row {
                            if !inserting && spec.exclude_from_update.contains(&column.as_str()) {
                                continue;
                            }
                            entry.insert(column.clone(), value.clone());
                        }
                    }
                }
                StorageOp::ScopedGuardedBump(spec) if spec.table == "channel_member" => {
                    let key = (text_of(&spec.scope_val), text_of(&spec.key_val));
                    let Some(entry) = self.rows.get_mut(&key) else {
                        continue;
                    };
                    let guard = entry.get(spec.guard_col).map(int_of).unwrap_or_default();
                    if spec.guard_val <= guard {
                        continue; // 前进守卫：重复/乱序帧不重复自增
                    }
                    let current = entry.get(spec.bump_col).map(int_of).unwrap_or_default();
                    entry.insert(
                        spec.bump_col.to_string(),
                        SqlValue::Integer(current + spec.bump_delta),
                    );
                    for (column, value) in &spec.set_cols {
                        entry.insert(column.clone(), value.clone());
                    }
                }
                _ => {}
            }
        }
    }

    fn read(&self, channel_id: &str, viewer: &str) -> ReplyBytes {
        let row: Row = self
            .rows
            .get(&(channel_id.to_string(), viewer.to_string()))
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .collect();
        ReplyBytes(rows_to_reply_bytes(&[row]))
    }

    fn unread(&self, channel_id: &str, viewer: &str) -> i64 {
        self.rows
            .get(&(channel_id.to_string(), viewer.to_string()))
            .and_then(|row| row.get("unread_count"))
            .map(int_of)
            .unwrap_or_default()
    }
}

fn text_of(value: &SqlValue) -> String {
    match value {
        SqlValue::Text(value) => value.clone(),
        other => panic!("expected text column, got {other:?}"),
    }
}

fn int_of(value: &SqlValue) -> i64 {
    match value {
        SqlValue::Integer(value) => *value,
        _ => 0,
    }
}

fn column_text(row: &Row, column: &str) -> String {
    row.iter()
        .find_map(|(name, value)| (name == column).then(|| text_of(value)))
        .unwrap_or_else(|| panic!("channel_member row missing {column}"))
}

// ─── Effect 探针 ───────────────────────────────────────────────────────────

fn emitted(effects: &[Effect]) -> Vec<Value> {
    effects
        .iter()
        .filter_map(|effect| {
            let Effect::Emit { event } = effect else {
                return None;
            };
            serde_json::from_slice(event.0.as_ref()).ok()
        })
        .collect()
}

fn http_corr(effects: &[Effect]) -> Option<Correlation> {
    effects.iter().find_map(|effect| match effect {
        Effect::Http { corr, .. } => Some(*corr),
        _ => None,
    })
}

fn persist_corr(effects: &[Effect]) -> Option<Correlation> {
    effects.iter().find_map(|effect| match effect {
        Effect::Persist { corr, .. } => Some(*corr),
        _ => None,
    })
}

/// 断言只做按主键的只读 Get，转发绝不改写来源行或来源频道行。
fn expect_get(effects: &[Effect], table: &str, key: &str) -> Correlation {
    effects
        .iter()
        .find_map(|effect| {
            let Effect::Persist { corr, ops } = effect else {
                return None;
            };
            let [StorageOp::Get(spec)] = ops.as_slice() else {
                return None;
            };
            (spec.table == table && text_of(&spec.key_val) == key).then_some(*corr)
        })
        .unwrap_or_else(|| panic!("missing read-only {table} lookup for {key}"))
}

fn atomic_commit(effects: &[Effect]) -> Option<(Correlation, &Vec<StorageOp>)> {
    effects.iter().find_map(|effect| {
        let Effect::PersistAtomic { corr, ops } = effect else {
            return None;
        };
        Some((*corr, ops))
    })
}

fn unread_bumps(ops: &[StorageOp]) -> Vec<i64> {
    ops.iter()
        .filter_map(|op| {
            let StorageOp::ScopedGuardedBump(spec) = op else {
                return None;
            };
            (spec.table == "channel_member"
                && spec.bump_col == "unread_count"
                && spec.bump_delta > 0)
                .then_some(spec.bump_delta)
        })
        .collect()
}

fn unread_absolute_writes(ops: &[StorageOp]) -> Vec<i64> {
    ops.iter()
        .filter_map(|op| {
            let StorageOp::BatchUpsert(spec) = op else {
                return None;
            };
            if spec.table != "channel_member" || spec.exclude_from_update.contains(&"unread_count")
            {
                return None;
            }
            spec.rows.first().and_then(|row| {
                row.iter()
                    .find_map(|(name, value)| (name == "unread_count").then(|| int_of(value)))
            })
        })
        .collect()
}

// ─── 输入构造 ──────────────────────────────────────────────────────────────

fn source_row_in(
    channel: u64,
    id: u64,
    user_id: &str,
    author: &str,
    message: &str,
    create_at: i64,
) -> Value {
    json!({
        "id": sid(id),
        "temporary_id": format!("source-{}", sid(id)),
        "channel_id": cid(channel),
        "user_id": user_id,
        "type": "TEXT",
        "message": message,
        "simple_message": message,
        "props": "{}",
        "mentions": "[]",
        "viewers": "[\"all\"]",
        "user_snapshot": json!({"userName": author}).to_string(),
        "create_at": create_at,
    })
}

fn source_row(id: u64, user_id: &str, author: &str, message: &str, create_at: i64) -> Value {
    source_row_in(SOURCE_CHANNEL, id, user_id, author, message, create_at)
}

fn relay_tick(mode: &str, source_ids: &[String], targets: &[String]) -> Tick {
    let payload = json!({
        "post_ids": source_ids,
        "target_channel_ids": targets,
        "mode": mode,
        "req_id": REQ_ID,
    });
    let bytes = serde_json::to_vec(&payload).expect("payload encodes");
    helix_im::client_api::build_command("im_relay_messages", &bytes)
        .map(Tick::Command)
        .expect("public relay command builds")
}

/// 驱动转发直到 createPosts 出站，返回真实 HTTP body。merge 模式额外闭合来源频道标题读取。
fn drive_forward(
    harness: &mut Harness,
    rows: &[Value],
    targets: &[String],
    mode: &str,
    now_ms: u64,
) -> Value {
    let source_ids: Vec<String> = rows
        .iter()
        .map(|row| row["id"].as_str().expect("source id").to_string())
        .collect();
    harness.step(relay_tick(mode, &source_ids, targets), now_ms);
    for (index, source_id) in source_ids.iter().enumerate() {
        let corr = expect_get(harness.sink.as_slice(), "message", source_id);
        let body = serde_json::to_vec(&json!([rows[index]])).expect("row encodes");
        harness.step(
            port_reply_with_body(corr.raw(), Bytes::from(body)),
            now_ms + index as u64 + 1,
        );
    }
    if mode == "merge" || mode == "merged" {
        let corr = expect_get(harness.sink.as_slice(), "channel", &cid(SOURCE_CHANNEL));
        let body = serde_json::to_vec(&json!([{
            "id": cid(SOURCE_CHANNEL),
            "display_name": SOURCE_TITLE
        }]))
        .expect("channel row encodes");
        harness.step(
            port_reply_with_body(corr.raw(), Bytes::from(body)),
            now_ms + source_ids.len() as u64 + 1,
        );
    }
    harness
        .last_http_body()
        .unwrap_or_else(|| panic!("{mode} forward must reach createPosts"))
}

/// 把 createPosts 的一条 post 补成 Go WS `post` authority。
fn authority(post: &Value, channel_id: &str, id: u64, seq: u64, create_at: i64) -> Value {
    let mut post = post.clone();
    post["id"] = json!(sid(id));
    post["channelId"] = json!(channel_id);
    post["createAt"] = json!(create_at);
    post["updateAt"] = json!(create_at);
    json!({
        "action": "post",
        "tracing": {"csesTrackId": REQ_ID},
        "data": {"event_seq": seq, "channelId": channel_id, "post": post}
    })
}

struct Applied {
    committed: bool,
    bumps: Vec<i64>,
    absolute: Vec<i64>,
    events: Vec<Value>,
}

/// 完整跑 authority → PersistAtomic → 读回 → Emit，并把写集应用到迷你驱动。
fn apply_authority(
    harness: &mut Harness,
    table: &mut MemberTable,
    viewer: &str,
    channel_id: &str,
    authority: &Value,
    now_ms: u64,
) -> Applied {
    let inbound = serde_json::to_vec(authority).expect("authority encodes");
    harness.step(Tick::Inbound(InboundBytes(Bytes::from(inbound))), now_ms);
    let Some((commit_corr, ops)) = atomic_commit(harness.sink.as_slice()) else {
        return Applied {
            committed: false,
            bumps: Vec::new(),
            absolute: Vec::new(),
            events: emitted(harness.sink.as_slice()),
        };
    };
    let bumps = unread_bumps(ops);
    let absolute = unread_absolute_writes(ops);
    table.apply(ops);

    harness.step(port_reply_ok(commit_corr.raw()), now_ms + 1);
    let readback = persist_corr(harness.sink.as_slice())
        .expect("committed authority must read the viewer's member row back");
    harness.step(
        Tick::PortReply {
            corr: readback,
            outcome: helix_core::tick::PortOutcome::Ok(table.read(channel_id, viewer)),
        },
        now_ms + 2,
    );
    Applied {
        committed: true,
        bumps,
        absolute,
        events: emitted(harness.sink.as_slice()),
    }
}

fn three_sources() -> [Value; 3] {
    [
        source_row(
            SOURCE_A,
            "source-user-a",
            "Actor",
            "第一条源消息",
            CREATE_AT - 30,
        ),
        source_row(
            SOURCE_B,
            "source-user-b",
            "Observer",
            "第二条源消息",
            CREATE_AT - 20,
        ),
        source_row(
            SOURCE_C,
            "source-user-c",
            "Reviewer",
            "第三条源消息",
            CREATE_AT - 10,
        ),
    ]
}

// ─── Gate 断言 ─────────────────────────────────────────────────────────────

/// 参考包的 canonical mode token 必须被内核接受，不得因内核旧别名反向削弱目标载荷。
#[test]
fn canonical_merge_mode_token_from_reference_is_accepted() {
    let sample = reference_json("inbound-sample.json");
    let payload = &sample["rust_entry_inputs"][0]["value"]["payload"];
    assert_eq!(
        payload["mode"], "merge",
        "参考包是线型权威：MV3-G03b 的 mode token 必须是 merge"
    );
    let mode = payload["mode"].as_str().expect("mode token").to_string();

    let mut harness = Harness::new_with_identity(&[TARGET_A], SENDER, "mv3-g03b", "", "", "");
    let rows = three_sources();
    let body = drive_forward(
        &mut harness,
        &rows,
        &[cid(TARGET_A)],
        &mode,
        CREATE_AT as u64,
    );
    assert_eq!(
        body["posts"][0]["type"], "MULTIPLY",
        "canonical merge token 必须走合并装配，而不是被当成未知 mode 拒绝"
    );
}

/// merge 模式把任意条数的源消息收敛为**单条** MULTIPLY，并采用参考包声明的摘要文案。
#[test]
fn merge_mode_collapses_every_source_into_exactly_one_multiply_post() {
    let mut harness = Harness::new_with_identity(&[TARGET_A], SENDER, "mv3-g03b", "", "", "");
    let rows = three_sources();
    let body = drive_forward(
        &mut harness,
        &rows,
        &[cid(TARGET_A)],
        "merge",
        CREATE_AT as u64,
    );

    let posts = body["posts"].as_array().expect("posts array");
    assert_eq!(
        posts.len(),
        1,
        "3 条源消息在 merge 模式下必须收敛为 1 条，不得退化为逐条转发"
    );
    let post = &posts[0];
    assert_eq!(post["type"], "MULTIPLY");
    let expected_simple = reference_json("outbound-sample.json")["samples"][0]["rust"]["data"]
        ["simpleMessage"]
        .clone();
    assert_eq!(expected_simple, "[聊天记录]");
    assert_eq!(
        post["simpleMessage"], expected_simple,
        "合并消息摘要必须与参考包一致"
    );
    assert_eq!(post["props"]["sourceChannelId"], cid(SOURCE_CHANNEL));
    assert_eq!(post["props"]["sourceChannelName"], SOURCE_TITLE);
    assert_eq!(
        post["props"]["forwardTitle"],
        format!("{SOURCE_TITLE}的聊天记录"),
        "合并标题必须来自已读取的来源频道权威，而不是 renderer 猜测"
    );
    assert_eq!(
        post["temporaryId"],
        format!("relay-{REQ_ID}-0"),
        "合并转发只分配一个对账键（INV-02）"
    );
}

/// mergeMessage 必须逐条保留源消息顺序与稳定 identity。
#[test]
fn merge_message_preserves_source_order_and_identity() {
    let mut harness = Harness::new_with_identity(&[TARGET_A], SENDER, "mv3-g03b", "", "", "");
    let rows = three_sources();
    let body = drive_forward(
        &mut harness,
        &rows,
        &[cid(TARGET_A)],
        "merge",
        CREATE_AT as u64,
    );

    let encoded = body["posts"][0]["props"]["mergeMessage"]
        .as_str()
        .expect("mergeMessage must be a wire string");
    let items: Vec<Value> = serde_json::from_str(encoded).expect("mergeMessage decodes to array");
    assert_eq!(items.len(), 3);
    assert_eq!(
        items
            .iter()
            .map(|item| item["id"].as_str().unwrap_or_default().to_string())
            .collect::<Vec<_>>(),
        vec![sid(SOURCE_A), sid(SOURCE_B), sid(SOURCE_C)],
        "合并项顺序必须严格等于用户选择的源消息顺序"
    );
    assert_eq!(
        items
            .iter()
            .map(|item| item["message"].as_str().unwrap_or_default().to_string())
            .collect::<Vec<_>>(),
        vec!["第一条源消息", "第二条源消息", "第三条源消息"]
    );
    assert_eq!(
        items
            .iter()
            .map(|item| item["userId"].as_str().unwrap_or_default().to_string())
            .collect::<Vec<_>>(),
        vec!["source-user-a", "source-user-b", "source-user-c"],
        "合并项必须保留各自作者，不得统一成转发者"
    );
    assert_eq!(
        items
            .iter()
            .map(|item| item["createAt"].as_i64().unwrap_or_default())
            .collect::<Vec<_>>(),
        vec![CREATE_AT - 30, CREATE_AT - 20, CREATE_AT - 10]
    );
}

/// 数量语义对撞：同样 3 条源消息，item 模式接收者 +3，merge 模式只 +1。
#[test]
fn merge_accrues_one_unread_while_item_accrues_one_per_source() {
    let rows = three_sources();
    let targets = [cid(TARGET_A)];

    // merge：单条 MULTIPLY → 接收者 +1。
    let mut merge_sender = Harness::new_with_identity(&[TARGET_A], SENDER, "mv3-g03b", "", "", "");
    let merge_body = drive_forward(
        &mut merge_sender,
        &rows,
        &targets,
        "merge",
        CREATE_AT as u64,
    );
    let merge_posts = merge_body["posts"].as_array().expect("posts").clone();

    let mut merge_receiver =
        Harness::new_with_identity(&[TARGET_A], RECEIVER, "mv3-g03b", "", "", "");
    let mut merge_table = MemberTable::default();
    let mut merge_bumps = 0_i64;
    for (index, post) in merge_posts.iter().enumerate() {
        let frame = authority(
            post,
            &cid(TARGET_A),
            300 + index as u64,
            index as u64 + 1,
            CREATE_AT + 1 + index as i64,
        );
        let applied = apply_authority(
            &mut merge_receiver,
            &mut merge_table,
            RECEIVER,
            &cid(TARGET_A),
            &frame,
            CREATE_AT as u64 + 100 + index as u64 * 10,
        );
        assert!(applied.committed);
        merge_bumps += applied.bumps.iter().sum::<i64>();
    }

    // item：3 条独立消息 → 接收者 +3。
    let mut item_sender = Harness::new_with_identity(&[TARGET_A], SENDER, "mv3-g03b", "", "", "");
    let item_body = drive_forward(&mut item_sender, &rows, &targets, "item", CREATE_AT as u64);
    let item_posts = item_body["posts"].as_array().expect("posts").clone();

    let mut item_receiver =
        Harness::new_with_identity(&[TARGET_A], RECEIVER, "mv3-g03b", "", "", "");
    let mut item_table = MemberTable::default();
    let mut item_bumps = 0_i64;
    for (index, post) in item_posts.iter().enumerate() {
        let frame = authority(
            post,
            &cid(TARGET_A),
            400 + index as u64,
            index as u64 + 1,
            CREATE_AT + 1 + index as i64,
        );
        let applied = apply_authority(
            &mut item_receiver,
            &mut item_table,
            RECEIVER,
            &cid(TARGET_A),
            &frame,
            CREATE_AT as u64 + 200 + index as u64 * 10,
        );
        assert!(applied.committed);
        item_bumps += applied.bumps.iter().sum::<i64>();
    }

    assert_eq!(merge_posts.len(), 1, "merge 必须只产出 1 条消息");
    assert_eq!(item_posts.len(), 3, "item 必须逐条产出 3 条消息");
    assert_eq!(merge_bumps, 1, "merge 模式接收者未读只 +1");
    assert_eq!(item_bumps, 3, "item 模式接收者未读 +N");
    assert_eq!(merge_table.unread(&cid(TARGET_A), RECEIVER), 1);
    assert_eq!(item_table.unread(&cid(TARGET_A), RECEIVER), 3);
    assert!(
        merge_bumps < item_bumps,
        "两种模式的未读数量语义必须可区分；若相等说明 mode 已不再是唯一分流字段"
    );
}

/// 同一条 authority 重复到达必须完全幂等：无第二次提交、无第二次事件、未读不动。
#[test]
fn duplicate_merged_authority_is_idempotent() {
    let mut sender = Harness::new_with_identity(&[TARGET_A], SENDER, "mv3-g03b", "", "", "");
    let rows = three_sources();
    let body = drive_forward(
        &mut sender,
        &rows,
        &[cid(TARGET_A)],
        "merge",
        CREATE_AT as u64,
    );
    let post = body["posts"][0].clone();
    let frame = authority(&post, &cid(TARGET_A), 500, 1, CREATE_AT + 1);

    let mut receiver = Harness::new_with_identity(&[TARGET_A], RECEIVER, "mv3-g03b", "", "", "");
    let mut table = MemberTable::default();

    let first = apply_authority(
        &mut receiver,
        &mut table,
        RECEIVER,
        &cid(TARGET_A),
        &frame,
        CREATE_AT as u64 + 100,
    );
    assert!(first.committed);
    assert_eq!(first.bumps, [1]);
    assert_eq!(table.unread(&cid(TARGET_A), RECEIVER), 1);
    assert!(first
        .events
        .iter()
        .any(|event| event["event"] == "im:post:received"));

    let replay = apply_authority(
        &mut receiver,
        &mut table,
        RECEIVER,
        &cid(TARGET_A),
        &frame,
        CREATE_AT as u64 + 200,
    );
    assert!(
        !replay.committed,
        "重复 event_seq 必须在 gate 处被丢弃，不得再次进入原子提交"
    );
    assert!(
        replay.events.is_empty(),
        "重复投递不得再次发布 post/channel 终态"
    );
    assert_eq!(
        table.unread(&cid(TARGET_A), RECEIVER),
        1,
        "重复投递不得重复累加未读"
    );
}

/// 双 persona：发送者绝对置零，接收者独立累加，禁止跨 persona 复制（INV-12）。
#[test]
fn merged_forward_diverges_between_sender_and_receiver_personas() {
    let mut sender = Harness::new_with_identity(&[TARGET_A], SENDER, "mv3-g03b", "", "", "");
    let rows = three_sources();
    let body = drive_forward(
        &mut sender,
        &rows,
        &[cid(TARGET_A)],
        "merge",
        CREATE_AT as u64,
    );
    let post = body["posts"][0].clone();
    let frame = authority(&post, &cid(TARGET_A), 600, 1, CREATE_AT + 1);

    let mut sender_table = MemberTable::default();
    let sender_applied = apply_authority(
        &mut sender,
        &mut sender_table,
        SENDER,
        &cid(TARGET_A),
        &frame,
        CREATE_AT as u64 + 100,
    );
    assert!(sender_applied.committed);
    assert!(
        sender_applied.bumps.is_empty(),
        "发送者视角禁止 unread 自增"
    );
    assert!(
        sender_applied.absolute.is_empty(),
        "发送者视角不得覆盖既有 unread_count"
    );

    let mut receiver = Harness::new_with_identity(&[TARGET_A], RECEIVER, "mv3-g03b", "", "", "");
    let mut receiver_table = MemberTable::default();
    let receiver_applied = apply_authority(
        &mut receiver,
        &mut receiver_table,
        RECEIVER,
        &cid(TARGET_A),
        &frame,
        CREATE_AT as u64 + 100,
    );
    assert!(receiver_applied.committed);
    assert_eq!(receiver_applied.bumps, [1]);

    let sender_channel = sender_applied
        .events
        .iter()
        .find(|event| event["event"] == "im:channel:update")
        .expect("sender channel projection");
    let receiver_channel = receiver_applied
        .events
        .iter()
        .find(|event| event["event"] == "im:channel:update")
        .expect("receiver channel projection");
    assert_eq!(sender_channel["data"]["unreadCount"], 0);
    assert_eq!(receiver_channel["data"]["unreadCount"], 1);
    assert_eq!(sender_table.unread(&cid(TARGET_A), RECEIVER), 0);
    assert_eq!(receiver_table.unread(&cid(TARGET_A), SENDER), 0);

    // 两个 persona 都必须拿到 bounded forwardDetail，且 raw mergeMessage 不跨事件边界。
    for (label, applied) in [("sender", &sender_applied), ("receiver", &receiver_applied)] {
        let received = applied
            .events
            .iter()
            .find(|event| event["event"] == "im:post:received")
            .unwrap_or_else(|| panic!("{label} must observe the merged post"));
        assert_eq!(received["data"]["type"], "MULTIPLY", "{label}");
        assert_eq!(
            received["data"]["forwardDetail"]["itemCount"], 3,
            "{label} 的合并投影必须还原 3 条源消息"
        );
        assert_eq!(
            received["data"]["forwardDetail"]["sourceChannelName"], SOURCE_TITLE,
            "{label}"
        );
        assert!(
            received["data"].get("props").is_none(),
            "{label}: raw props.mergeMessage 不得跨事件边界外泄"
        );
    }
}

/// 合并转发只接受同一来源频道；跨频道选择必须 fail closed 且绝不出站。
#[test]
fn merged_forward_requires_a_single_source_channel() {
    let mut harness = Harness::new_with_identity(&[TARGET_A], SENDER, "mv3-g03b", "", "", "");
    let rows = [
        source_row(
            SOURCE_A,
            "source-user-a",
            "Actor",
            "第一条源消息",
            CREATE_AT - 30,
        ),
        source_row_in(
            OTHER_SOURCE_CHANNEL,
            SOURCE_B,
            "source-user-b",
            "Observer",
            "别的频道",
            CREATE_AT - 20,
        ),
    ];
    let source_ids: Vec<String> = rows
        .iter()
        .map(|row| row["id"].as_str().expect("source id").to_string())
        .collect();
    harness.step(
        relay_tick("merge", &source_ids, &[cid(TARGET_A)]),
        CREATE_AT as u64,
    );
    for (index, source_id) in source_ids.iter().enumerate() {
        let corr = expect_get(harness.sink.as_slice(), "message", source_id);
        let body = serde_json::to_vec(&json!([rows[index]])).expect("row encodes");
        harness.step(
            port_reply_with_body(corr.raw(), Bytes::from(body)),
            CREATE_AT as u64 + index as u64 + 1,
        );
    }

    assert!(
        http_corr(harness.sink.as_slice()).is_none(),
        "跨来源频道的合并转发禁止出站 createPosts"
    );
    let batch = emitted(harness.sink.as_slice())
        .into_iter()
        .find(|event| event["event"] == "im:posts:batch-result")
        .expect("跨频道合并必须发布可观察的批失败");
    assert_eq!(batch["data"]["batchStatus"], "failed");
    assert_eq!(batch["data"]["targets"][0]["channelId"], cid(TARGET_A));
    assert_eq!(
        batch["data"]["targets"][0]["deliveryStatus"],
        "not-applicable"
    );
}
