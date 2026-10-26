//! MV3-G12b：一级回复分支分页、锚点隔离与乱序回放。

use bytes::Bytes;
use helix_core::effect::Effect;
use helix_core::tick::{AppCommand, PortOutcome, ReplyBytes};
use helix_core::{Correlation, EffectSink, Module, Tick};
use helix_im::module::ImConfig;
use helix_im::query::LocalStoreMode;
use helix_im::render_ready_replies::{
    project_replies_page, ReplyProjectionMode, ReplyProjectionRequest,
};
use helix_im::state::ChannelId;
use helix_im::ImModule;
use serde_json::{json, Value};

#[path = "../../../common.rs"]
mod common;

const API_BASE: &str = "http://g12b.invalid/api/cses";
const DEFAULT_BASE: &str = "http://g12b.invalid";
const CHANNEL: &str = "chfixx0000000000000000001b";
const ROOT: &str = "root-g12b";
const FIRST_A: &str = "first-a-g12b";
const FIRST_B: &str = "first-b-g12b";
const VIEWER: &str = "viewer-g12b";

/// 从 outbound batch 取唯一 HTTP effect，固定 I/O 前的 wire 断言入口。
fn one_http(effects: Vec<Effect>) -> helix_core::effect::HttpRequest {
    let mut requests = effects.into_iter().filter_map(|effect| match effect {
        Effect::Http { req, .. } => Some(req),
        _ => None,
    });
    let request = requests.next().expect("reply branch must issue one HTTP");
    assert!(
        requests.next().is_none(),
        "reply branch must issue one HTTP"
    );
    request
}

/// 按 Go `/posts/getReplyBranch` 的 HasMorePage 形状构造 authority body。
fn branch_body(first_level: &str, id: &str, has_more: bool) -> Value {
    json!({
        "status": "SUCCESS",
        "data": [
            {
                "id": id,
                "temporaryId": format!("tmp-{id}"),
                "channelId": CHANNEL,
                "userId": VIEWER,
                "type": "TEXT",
                "message": id,
                "replyId": ROOT,
                "replyRootId": ROOT,
                "replyFirstLevelId": first_level,
                "parentId": first_level,
                "viewers": ["all"],
                "createAt": 1700000002000_i64
            }
        ],
        "hasMore": has_more
    })
}

/// 将裸 JSON 包装为 Helix HTTP port reply envelope。
fn http_envelope(body: &Value) -> Bytes {
    let raw = serde_json::to_vec(body).expect("fixture JSON must serialize");
    let envelope = json!({
        "status": 200,
        "headers": {},
        "body": base64_std(&raw)
    });
    Bytes::from(serde_json::to_vec(&envelope).expect("HTTP envelope must serialize"))
}

/// 仅用于测试 envelope 的无依赖标准 Base64 编码。
fn base64_std(bytes: &[u8]) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut output = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let first = u32::from(chunk[0]);
        let second = chunk.get(1).copied().map_or(0, u32::from);
        let third = chunk.get(2).copied().map_or(0, u32::from);
        let encoded = (first << 16) | (second << 8) | third;
        output.push(TABLE[((encoded >> 18) & 63) as usize] as char);
        output.push(TABLE[((encoded >> 12) & 63) as usize] as char);
        output.push(
            chunk
                .get(1)
                .map_or('=', |_| TABLE[((encoded >> 6) & 63) as usize] as char),
        );
        output.push(
            chunk
                .get(2)
                .map_or('=', |_| TABLE[(encoded & 63) as usize] as char),
        );
    }
    output
}

/// 通过公开 `ImModule::handle` 驱动真实 branch command/PortReply 链。
struct Replay {
    module: ImModule,
    sink: EffectSink,
}

impl Replay {
    /// 保留 Session 本地存储与既有端点，只通过公共提交 seam 补可信归属。
    fn new() -> Self {
        let mut module = ImModule::new_with_local_store(
            ImConfig {
                api_base_url: API_BASE.to_string(),
                default_api_base_url: DEFAULT_BASE.to_string(),
                auth_user_id: VIEWER.to_string(),
                company_id: "company-g12b".to_string(),
                ..ImConfig::default()
            },
            LocalStoreMode::Session,
        );
        module.register_channel(ChannelId::from_str(CHANNEL).expect("channel fixture"), 0);
        let mut harness = common::Harness {
            module,
            sink: EffectSink::new(),
            effect_log: Vec::new(),
        };
        harness.seed_channel_scope(CHANNEL, "company-g12b", VIEWER);
        harness.sink.clear();
        Self {
            module: harness.module,
            sink: harness.sink,
        }
    }

    /// 执行一个 Tick 并覆盖当前 effect batch。
    fn step(&mut self, tick: Tick, now_ms: u64) -> Result<(), String> {
        self.sink.clear();
        self.module
            .handle(&tick, now_ms, &mut self.sink)
            .map_err(|error| error.to_string())
    }

    /// 发起 branch command，返回与请求关联的 HTTP correlation。
    fn command(
        &mut self,
        first_level: &str,
        req_id: &str,
        page_number: u64,
    ) -> Result<Correlation, String> {
        let payload = json!({
            "channel_id": CHANNEL,
            "reply_first_level_id": first_level,
            "root_message_id": ROOT,
            "page_number": page_number,
            "page_size": 20,
            "req_id": req_id
        });
        self.step(
            Tick::Command(AppCommand::new(
                "im_get_reply_branch",
                serde_json::to_vec(&payload).map_err(|error| error.to_string())?,
            )),
            page_number,
        )?;
        self.sink
            .as_slice()
            .iter()
            .find_map(|effect| match effect {
                Effect::Http { corr, .. } => Some(*corr),
                _ => None,
            })
            .ok_or_else(|| "branch command did not emit HTTP".to_string())
    }

    /// 回灌一个 authority branch body。
    fn reply(&mut self, corr: Correlation, body: Value) -> Result<(), String> {
        self.step(
            Tick::PortReply {
                corr,
                outcome: PortOutcome::Ok(ReplyBytes(http_envelope(&body))),
            },
            10,
        )
    }

    /// 解码当前 batch 中的 domain events。
    fn events(&self) -> Vec<Value> {
        self.sink
            .as_slice()
            .iter()
            .filter_map(|effect| match effect {
                Effect::Emit { event } => serde_json::from_slice(event.0.as_ref()).ok(),
                _ => None,
            })
            .collect()
    }
}

#[test]
fn g12b_outbound_preserves_branch_page_and_header_req_id() {
    let payload = serde_json::to_vec(&json!({
        "reply_first_level_id": FIRST_A,
        "page_number": 2,
        "page_size": 60,
        "req_id": "req-g12b-wire",
        "channel_id": CHANNEL,
        "root_message_id": ROOT
    }))
    .unwrap();
    let request = one_http(
        helix_im::outbound::handle_outbound(
            "im_get_reply_branch",
            &payload,
            API_BASE,
            DEFAULT_BASE,
            None,
            Correlation::from_raw(21),
        )
        .unwrap(),
    );
    assert_eq!(request.url, format!("{API_BASE}/posts/getReplyBranch"));
    let body: Value = serde_json::from_slice(request.body.as_ref().unwrap()).unwrap();
    assert_eq!(body["replyFirstLevelId"], FIRST_A);
    assert_eq!(body["pageNumber"], 2);
    assert_eq!(body["pageSize"], 60);
    assert!(body.get("reqId").is_none());
    assert_eq!(
        request
            .headers
            .iter()
            .find(|(key, _)| key == "Cses-Track-Id")
            .map(|(_, value)| value.as_str()),
        Some("req-g12b-wire")
    );

    let defaults = serde_json::to_vec(&json!({
        "reply_first_level_id": FIRST_A,
        "req_id": "req-g12b-default"
    }))
    .unwrap();
    let body: Value = serde_json::from_slice(
        one_http(
            helix_im::outbound::handle_outbound(
                "im_get_reply_branch",
                &defaults,
                API_BASE,
                DEFAULT_BASE,
                None,
                Correlation::from_raw(22),
            )
            .unwrap(),
        )
        .body
        .as_ref()
        .unwrap(),
    )
    .unwrap();
    assert_eq!(body["pageNumber"], 1);
    assert_eq!(body["pageSize"], 20);
}

#[test]
fn g12b_rejects_missing_anchor_and_invalid_page_before_http() {
    for payload in [
        json!({"req_id": "req-missing"}),
        json!({"reply_first_level_id": FIRST_A, "req_id": "req-zero", "page_number": 0}),
        json!({"reply_first_level_id": FIRST_A, "req_id": "req-too-large", "page_size": 61}),
    ] {
        let bytes = serde_json::to_vec(&payload).unwrap();
        assert!(
            helix_im::outbound::handle_outbound(
                "im_get_reply_branch",
                &bytes,
                API_BASE,
                DEFAULT_BASE,
                None,
                Correlation::from_raw(23),
            )
            .is_err(),
            "invalid branch request must stop before I/O: {payload}"
        );
    }
}

#[test]
fn g12b_projection_uses_first_level_anchor_and_real_go_page() {
    let request = ReplyProjectionRequest {
        req_id: "req-g12b-project".to_string(),
        channel_id: CHANNEL.to_string(),
        root_hint: FIRST_A.to_string(),
        mode: ReplyProjectionMode::Append,
        revision: 1,
        viewer_user_id: VIEWER.to_string(),
        page_number: 1,
        page_size: 20,
    };
    let body = json!({
        "status": "SUCCESS",
        "data": [
            {
                "id": "branch-a-1",
                "temporaryId": "tmp-branch-a-1",
                "channelId": CHANNEL,
                "replyId": ROOT,
                "replyRootId": ROOT,
                "replyFirstLevelId": FIRST_A,
                "viewers": [VIEWER]
            },
            {
                "id": "branch-b-leak",
                "temporaryId": "tmp-branch-b-leak",
                "channelId": CHANNEL,
                "replyId": ROOT,
                "replyRootId": ROOT,
                "replyFirstLevelId": FIRST_B,
                "viewers": [VIEWER]
            },
            {
                "id": "branch-a-duplicate",
                "temporaryId": "tmp-branch-a-1",
                "channelId": CHANNEL,
                "replyId": ROOT,
                "replyRootId": ROOT,
                "replyFirstLevelId": FIRST_A,
                "viewers": [VIEWER]
            },
            {
                "id": "branch-a-hidden",
                "temporaryId": "tmp-branch-a-hidden",
                "channelId": CHANNEL,
                "replyId": ROOT,
                "replyRootId": ROOT,
                "replyFirstLevelId": FIRST_A,
                "viewers": ["other-viewer"]
            }
        ],
        "hasMore": true
    });
    let result = project_replies_page(&request, &body, VIEWER).unwrap();
    assert_eq!(result.req_id, "req-g12b-project");
    assert_eq!(result.replies.len(), 1);
    assert_eq!(result.replies[0]["id"], "branch-a-1");
    assert!(result.root_post.is_null());
    assert!(result.has_more);
    assert_eq!(result.page_number, 1);
    assert_eq!(result.page_size, 20);
}

#[test]
fn g12b_from_command_prefers_branch_anchor_over_root_hint() {
    let payload = serde_json::to_vec(&json!({
        "reply_first_level_id": FIRST_A,
        "root_message_id": ROOT,
        "channel_id": CHANNEL,
        "page_number": 2,
        "page_size": 20,
        "req_id": "req-g12b-context"
    }))
    .unwrap();
    let request = ReplyProjectionRequest::from_command("im_get_reply_branch", &payload, 4, VIEWER)
        .expect("branch context must be admitted");
    assert_eq!(request.root_hint, FIRST_A);
    assert_eq!(request.page_number, 2);
    assert_eq!(request.page_size, 20);
}

#[test]
fn g12b_real_chain_isolates_branch_and_rejects_old_page() -> Result<(), String> {
    let mut replay = Replay::new();
    let branch_a_page_1 = replay.command(FIRST_A, "req-a-page-1", 1)?;
    let branch_a_page_2 = replay.command(FIRST_A, "req-a-page-2", 2)?;
    let branch_b_page_1 = replay.command(FIRST_B, "req-b-page-1", 1)?;

    replay.reply(
        branch_a_page_2,
        branch_body(FIRST_A, "branch-a-page-2", true),
    )?;
    let page_2_events = replay.events();
    assert_eq!(page_2_events.len(), 1);
    assert_eq!(page_2_events[0]["data"]["rootMessageId"], FIRST_A);
    assert_eq!(
        page_2_events[0]["data"]["replyIds"],
        json!(["branch-a-page-2"])
    );
    assert_eq!(page_2_events[0]["data"]["hasMore"], true);

    replay.reply(
        branch_b_page_1,
        branch_body(FIRST_B, "branch-b-page-1", false),
    )?;
    let branch_b_events = replay.events();
    assert_eq!(branch_b_events.len(), 1);
    assert_eq!(branch_b_events[0]["data"]["rootMessageId"], FIRST_B);
    assert_eq!(
        branch_b_events[0]["data"]["replyIds"],
        json!(["branch-b-page-1"])
    );

    replay.reply(
        branch_a_page_1,
        branch_body(FIRST_A, "branch-a-page-1-old", false),
    )?;
    assert!(
        replay.events().is_empty(),
        "late lower revision must not replace the newer branch page"
    );
    Ok(())
}

#[test]
fn g12b_empty_branch_is_a_terminal_empty_result() -> Result<(), String> {
    let mut replay = Replay::new();
    let corr = replay.command(FIRST_A, "req-empty-branch", 1)?;
    replay.reply(
        corr,
        json!({"status": "SUCCESS", "data": [], "hasMore": false}),
    )?;
    let events = replay.events();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["data"]["rootMessageId"], FIRST_A);
    assert_eq!(events[0]["data"]["replyIds"], json!([]));
    assert_eq!(events[0]["data"]["hasMore"], false);
    Ok(())
}
