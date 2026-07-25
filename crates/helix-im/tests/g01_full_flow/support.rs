use bytes::Bytes;
use helix_core::effect::{Correlation, Effect, StorageOp};
use helix_core::tick::{AppCommand, PortOutcome, ReplyBytes};
use helix_core::{EffectSink, Module, Tick};
use helix_im::module::ImConfig;
use helix_im::ImModule;
use serde_json::{json, Value};

pub const CHANNEL_ID: &str = "chflow00000000000000000001";
pub const ACTOR_ID: &str = "g01-flow-actor";
pub const TEAM_ID: &str = "g01-flow-team";
pub const REQUEST_ID: &str = "g01-flow-request";
pub const TEXT: &str = "g01 complete flow";
pub const CREATED_AT: i64 = 1_700_000_100_000;

pub struct Replay {
    pub module: ImModule,
    pub sink: EffectSink,
}

impl Replay {
    /// 创建带稳定身份和端点的确定性 Helix 回放实例。
    pub fn new() -> Self {
        Self {
            module: ImModule::new(ImConfig {
                api_base_url: "http://g01-flow.invalid".to_string(),
                default_api_base_url: "http://g01-flow.invalid".to_string(),
                auth_user_id: ACTOR_ID.to_string(),
                company_id: TEAM_ID.to_string(),
                user_name: "G01 Flow Actor".to_string(),
                ..ImConfig::default()
            }),
            sink: EffectSink::new(),
        }
    }

    /// 以单调注入时间推进一次 Tick，并只保留该 step 的 Effect。
    pub fn step(&mut self, tick: Tick, now_ms: u64) {
        self.sink.clear();
        self.module
            .handle(&tick, now_ms, &mut self.sink)
            .expect("G01 replay step must succeed");
    }

    /// 暴露当前 step 的 Effect 批次供因果断言使用。
    pub fn effects(&self) -> &[Effect] {
        self.sink.as_slice()
    }

    /// 按 endpoint suffix 取得唯一 HTTP correlation，避免依赖 Effect 顺序。
    pub fn only_http_corr(&self, suffix: &str) -> Correlation {
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

    /// 按 StorageOp 语义取得唯一 Persist correlation。
    pub fn only_persist_corr_with(&self, predicate: impl Fn(&[StorageOp]) -> bool) -> Correlation {
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

    /// 从当前 Effect 批次提取指定 Helix 事件信封。
    pub fn event(&self, name: &str) -> Option<Value> {
        self.effects().iter().find_map(|effect| {
            let Effect::Emit { event } = effect else {
                return None;
            };
            let envelope: Value = serde_json::from_slice(event.0.as_ref()).ok()?;
            (envelope["event"] == name).then_some(envelope)
        })
    }

    /// 提取当前批次中 `im:interaction:frame` 的纯 Frame data。
    pub fn frames(&self) -> Vec<Value> {
        self.effects()
            .iter()
            .filter_map(|effect| {
                let Effect::Emit { event } = effect else {
                    return None;
                };
                let envelope: Value = serde_json::from_slice(event.0.as_ref()).ok()?;
                (envelope["event"] == "im:interaction:frame")
                    .then(|| envelope.get("data").cloned())
                    .flatten()
            })
            .collect()
    }
}

/// 构造公开 command ingress，保持测试只走 `Tick::Command`。
pub fn command(name: &'static str, payload: Value) -> Tick {
    Tick::Command(AppCommand::new(
        name,
        serde_json::to_vec(&payload).expect("command payload serializes"),
    ))
}

/// 构造携带指定回包字节的成功 PortReply。
pub fn ok(corr: Correlation, bytes: Bytes) -> Tick {
    Tick::PortReply {
        corr,
        outcome: PortOutcome::Ok(ReplyBytes(bytes)),
    }
}

/// 按 host driver 的 ADR-007 形状封装 HTTP 响应。
pub fn http_envelope(body: Value) -> Bytes {
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

/// 建立 latest timeline attachment，使发送状态进入 typed Frame。
pub fn attach_timeline(replay: &mut Replay) {
    replay.step(
        command(
            "im_query_messages_by_channel",
            json!({
                "channel_id": CHANNEL_ID,
                "limit": 20,
                "windowToken": "latest",
            }),
        ),
        CREATED_AT as u64 + 2,
    );
    let local_corr = replay.only_persist_corr_with(|ops| {
        ops.iter()
            .any(|op| matches!(op, StorageOp::Scan(spec) if spec.table == "message"))
    });
    replay.step(
        ok(local_corr, Bytes::from_static(b"[]")),
        CREATED_AT as u64 + 3,
    );
    let remote_corr = replay.only_http_corr("/posts/getLatestPost");
    replay.step(
        ok(
            remote_corr,
            http_envelope(json!({"status": "SUCCESS", "data": []})),
        ),
        CREATED_AT as u64 + 4,
    );
    assert_frame_surface(replay, "message.timeline");

    replay.step(
        command(
            "__helix_request_surface_snapshot",
            json!({
                "surface": "message.timeline",
                "scope": {
                    "kind": "timeline",
                    "channelId": CHANNEL_ID,
                    "windowToken": "latest"
                },
                "causationId": "g01-flow-attach"
            }),
        ),
        CREATED_AT as u64 + 5,
    );
    let _attachment_read = replay.only_persist_corr_with(|ops| {
        ops.iter()
            .any(|op| matches!(op, StorageOp::Scan(spec) if spec.table == "message"))
    });
}

/// 判断一组 StorageOp 是否包含指定表的业务写。
pub fn has_table_write(ops: &[StorageOp], table: &str) -> bool {
    ops.iter().any(|op| match op {
        StorageOp::BatchUpsert(spec) => spec.table == table,
        StorageOp::BatchUpdate(spec) => spec.table == table,
        StorageOp::GuardedBump(spec) => spec.table == table,
        _ => false,
    })
}

/// 断言当前 Effect 批次含指定 V2 surface mutation。
pub fn assert_frame_surface(replay: &Replay, expected: &str) {
    let frames = replay.frames();
    assert!(
        frames.iter().any(|frame| {
            frame["schemaVersion"] == 2
                && frame["mutations"]
                    .as_array()
                    .is_some_and(|mutations| mutations.iter().any(|m| m["surface"] == expected))
        }),
        "missing V2 surface {expected}; frames={frames:?}"
    );
}

/// 以固定字母表编码测试 HTTP 信封，避免引入运行时依赖。
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
