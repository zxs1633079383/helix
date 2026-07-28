use std::fs;
use std::path::{Path, PathBuf};

use bytes::Bytes;
use helix_core::effect::{Correlation, Effect, SqlValue, StorageOp};
use helix_core::tick::{AppCommand, PortOutcome, ReplyBytes};
use helix_core::{EffectSink, Module, Tick};
use helix_im::interaction_frame::{
    InteractionFrameV2, SurfaceMutation, TimelineAnchorMode, TimelineDeltaOperationVm,
    TypedSurfaceMutation,
};
use helix_im::module::ImConfig;
use helix_im::projector::{build_online_interaction_frame, timeline::TimelineWindowRequest};
use helix_im::query::LocalStoreMode;
use helix_im::state::ChannelId;
use helix_im::ImModule;
use serde_json::{json, Value};

#[path = "../g01_shared/scenario.rs"]
#[allow(dead_code)]
mod shared;

pub use shared::sha256_hex;

/// 正式 Timeline Gate 的生产 ScenarioRunner，所有 Frame 只从 `ImModule::handle` 解码。
pub struct TimelineProductionRunner {
    module: ImModule,
    sink: EffectSink,
    channel_id: String,
    window_token: String,
    now_ms: u64,
}

/// 一次已完成 local miss、等待 Go context 回报的 production locate。
pub struct PendingLocate {
    http_corr: Correlation,
    target_message_id: String,
    has_older: bool,
    has_newer: bool,
}

impl TimelineProductionRunner {
    /// 创建带可信 runtime identity 与 Session store 的确定性 runner。
    pub fn new(channel_id: &str, window_token: &str) -> Self {
        let mut module = ImModule::new_with_local_store(
            ImConfig {
                api_base_url: "http://timeline-gate.invalid".to_string(),
                default_api_base_url: "http://timeline-gate.invalid".to_string(),
                auth_user_id: "timeline-gate-viewer".to_string(),
                company_id: "timeline-gate-company".to_string(),
                user_name: "Timeline Gate Viewer".to_string(),
                ..ImConfig::default()
            },
            LocalStoreMode::Session,
        );
        module.register_channel(
            ChannelId::from_str(channel_id).expect("formal channel id is valid"),
            0,
        );
        Self {
            module,
            sink: EffectSink::new(),
            channel_id: channel_id.to_string(),
            window_token: window_token.to_string(),
            now_ms: 1_700_200_000_000,
        }
    }

    /// 通过 local miss -> V3 latest HTTP -> cache PersistOk 建立 attached snapshot。
    pub fn attach_latest(&mut self, rows: Vec<Value>, page_size: u32) -> InteractionFrameV2 {
        self.command(
            "im_query_messages_by_channel",
            json!({
                "channel_id": self.channel_id,
                "windowToken": self.window_token,
                "pageSize": page_size,
            }),
        );
        let local = self.only_persist_corr();
        self.reply(local, Bytes::from_static(b"[]"));
        let http = self.only_http_corr("/posts/getLatestPost");
        self.reply(
            http,
            http_envelope(json!({
                "status": "SUCCESS",
                "data": {
                    "posts": rows,
                    "hasOlder": true,
                    "hasNewer": false
                }
            })),
        );
        if self.frames().is_empty() {
            let cache = self.only_persist_corr();
            self.reply(cache, Bytes::new());
        }
        self.only_frame()
    }

    /// 通过本地 miss、Go context、cache barrier 和 target readback 完成定位。
    pub fn locate(
        &mut self,
        target_message_id: &str,
        rows: Vec<Value>,
        page_size: u32,
        has_older: bool,
        has_newer: bool,
        request_id: &str,
    ) -> InteractionFrameV2 {
        let pending = self.start_locate(
            target_message_id,
            page_size,
            has_older,
            has_newer,
            request_id,
        );
        self.complete_locate(pending, rows)
    }

    /// 发起 locate intent 并推进到待回放的真实 HTTP correlation。
    pub fn start_locate(
        &mut self,
        target_message_id: &str,
        page_size: u32,
        has_older: bool,
        has_newer: bool,
        navigation_token: &str,
    ) -> PendingLocate {
        self.command(
            "im_locate_message",
            json!({
                "channel_id": self.channel_id,
                "message_id": target_message_id,
                "pageSize": page_size,
                "req_id": format!("locate-{navigation_token}"),
                "navigationToken": navigation_token,
            }),
        );
        let probe = self.only_persist_corr();
        self.reply(probe, Bytes::from_static(b"[]"));
        PendingLocate {
            http_corr: self.only_http_corr("/posts/postContext"),
            target_message_id: target_message_id.to_string(),
            has_older,
            has_newer,
        }
    }

    /// 乱序回放一个 locate HTTP，并穿过 cache barrier 与 target readback 取得 Frame。
    pub fn complete_locate(
        &mut self,
        pending: PendingLocate,
        rows: Vec<Value>,
    ) -> InteractionFrameV2 {
        let target_index = rows
            .iter()
            .position(|row| row["id"] == pending.target_message_id)
            .expect("locate page contains target");
        let target_row = rows[target_index].clone();
        self.reply(
            pending.http_corr,
            http_envelope(json!({
                "status": "SUCCESS",
                "data": {
                    "posts": rows,
                    "targetIndex": target_index,
                    "hasOlder": pending.has_older,
                    "hasNewer": pending.has_newer
                }
            })),
        );
        let cache = self.only_persist_corr();
        assert!(
            self.frames().is_empty(),
            "HTTP cannot emit before PersistOk"
        );
        self.reply(cache, Bytes::new());
        let readback = self.only_persist_corr();
        assert!(
            self.frames().is_empty(),
            "PersistOk only schedules readback"
        );
        self.reply(
            readback,
            Bytes::from(serde_json::to_vec(&vec![target_row]).expect("target row serializes")),
        );
        self.only_frame()
    }

    /// 通过 V3 composite newer cursor、cache barrier 和 readback 完成下拉页。
    pub fn load_newer(
        &mut self,
        anchor_message_id: &str,
        rows: Vec<Value>,
        page_size: u32,
        has_more: bool,
    ) -> InteractionFrameV2 {
        self.command(
            "im_load_newer_context",
            json!({
                "channel_id": self.channel_id,
                "anchor_post_id": anchor_message_id,
                "pageSize": page_size,
                "req_id": format!("newer-{anchor_message_id}"),
            }),
        );
        let http = self.only_http_corr("/posts/getPostsAfterIndex");
        let readback_row = rows.last().cloned();
        self.reply(
            http,
            http_envelope(json!({
                "status": "SUCCESS",
                "data": {
                    "posts": rows,
                    "hasMore": has_more
                }
            })),
        );
        if let Some(readback_row) = readback_row {
            let cache = self.only_persist_corr();
            assert!(
                self.frames().is_empty(),
                "HTTP cannot emit before PersistOk"
            );
            self.reply(cache, Bytes::new());
            let readback = self.only_persist_corr();
            assert!(
                self.frames().is_empty(),
                "PersistOk only schedules readback"
            );
            self.reply(
                readback,
                Bytes::from(
                    serde_json::to_vec(&vec![readback_row]).expect("readback row serializes"),
                ),
            );
        }
        self.only_frame()
    }

    /// 通过 V3 composite older cursor、cache barrier 和 readback 完成上拉页。
    pub fn load_older(
        &mut self,
        anchor_message_id: &str,
        rows: Vec<Value>,
        page_size: u32,
        has_more: bool,
    ) -> InteractionFrameV2 {
        self.command(
            "im_load_older_context",
            json!({
                "channel_id": self.channel_id,
                "anchor_post_id": anchor_message_id,
                "pageSize": page_size,
                "req_id": format!("older-{anchor_message_id}"),
            }),
        );
        let http = self.only_http_corr("/posts/getPostsAfterIndex");
        let readback_row = rows.first().cloned();
        self.reply(
            http,
            http_envelope(json!({
                "status": "SUCCESS",
                "data": {
                    "posts": rows,
                    "hasMore": has_more
                }
            })),
        );
        if let Some(readback_row) = readback_row {
            let cache = self.only_persist_corr();
            assert!(
                self.frames().is_empty(),
                "HTTP cannot emit before PersistOk"
            );
            self.reply(cache, Bytes::new());
            let readback = self.only_persist_corr();
            assert!(
                self.frames().is_empty(),
                "PersistOk only schedules readback"
            );
            self.reply(
                readback,
                Bytes::from(
                    serde_json::to_vec(&vec![readback_row]).expect("readback row serializes"),
                ),
            );
        }
        self.only_frame()
    }

    /// 通过生产 `im_send_message -> optimistic PersistOk -> timeline readback` 产生 append Delta。
    pub fn send_and_readback_append(&mut self, mut rows_asc: Vec<Value>) -> InteractionFrameV2 {
        self.command(
            "im_send_message",
            json!({
                "channel_id": self.channel_id,
                "text": "production appended message",
                "type": "TEXT",
                "props": {},
                "req_id": "formal-send-append",
            }),
        );
        let persist = self.only_persist_corr();
        let temporary_id = self
            .sink
            .as_slice()
            .iter()
            .find_map(|effect| match effect {
                Effect::Persist { ops, .. } => ops.iter().find_map(|op| match op {
                    StorageOp::BatchUpsert(spec) if spec.table == "message" => spec
                        .rows
                        .iter()
                        .flat_map(|row| row.iter())
                        .find_map(|(column, value)| {
                            (column == "temporary_id").then(|| match value {
                                SqlValue::Text(value) => value.clone(),
                                _ => String::new(),
                            })
                        }),
                    _ => None,
                }),
                _ => None,
            })
            .filter(|value| !value.is_empty())
            .expect("optimistic write exposes allocated temporary id");
        self.reply(persist, Bytes::new());
        let readback = self.only_persist_corr();
        let next_create_at = rows_asc
            .last()
            .and_then(|row| row.get("createAt"))
            .and_then(Value::as_i64)
            .unwrap_or_default()
            + 1;
        rows_asc.push(json!({
            "id": temporary_id,
            "temporaryId": temporary_id,
            "channelId": self.channel_id,
            "userId": "timeline-gate-viewer",
            "createAt": next_create_at,
            "type": "TEXT",
            "message": "production appended message",
            "sendStatus": "sending",
            "revoke": false,
            "props": {},
            "actionCapabilities": {}
        }));
        if rows_asc.len() > 60 {
            rows_asc.remove(0);
        }
        rows_asc.reverse();
        self.reply(
            readback,
            Bytes::from(serde_json::to_vec(&rows_asc).expect("append readback serializes")),
        );
        self.only_frame()
    }

    /// 执行公开 command 并清空上一 step 的 Effect。
    fn command(&mut self, name: &'static str, payload: Value) {
        self.step(Tick::Command(AppCommand::new(
            name,
            serde_json::to_vec(&payload).expect("formal command serializes"),
        )));
    }

    /// 回灌一个端口成功结果。
    fn reply(&mut self, corr: Correlation, body: Bytes) {
        self.step(Tick::PortReply {
            corr,
            outcome: PortOutcome::Ok(ReplyBytes(body)),
        });
    }

    /// 通过公开 Module trait 执行一步。
    fn step(&mut self, tick: Tick) {
        self.now_ms += 1;
        self.sink.clear();
        self.module
            .handle(&tick, self.now_ms, &mut self.sink)
            .expect("formal production handle succeeds");
    }

    /// 返回当前 batch 的唯一 Persist correlation。
    fn only_persist_corr(&self) -> Correlation {
        let corrs = self
            .sink
            .as_slice()
            .iter()
            .filter_map(|effect| match effect {
                Effect::Persist { corr, .. } => Some(*corr),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(corrs.len(), 1, "expected exactly one Persist");
        corrs[0]
    }

    /// 返回指定生产 endpoint 的唯一 HTTP correlation。
    fn only_http_corr(&self, endpoint: &str) -> Correlation {
        let corrs = self
            .sink
            .as_slice()
            .iter()
            .filter_map(|effect| match effect {
                Effect::Http { corr, req } if req.url.ends_with(endpoint) => Some(*corr),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(corrs.len(), 1, "expected exactly one HTTP {endpoint}");
        corrs[0]
    }

    /// 解码当前 batch 的全部生产 Frame。
    fn frames(&self) -> Vec<InteractionFrameV2> {
        self.sink
            .as_slice()
            .iter()
            .filter_map(|effect| match effect {
                Effect::Emit { event } => {
                    let envelope: Value =
                        serde_json::from_slice(event.0.as_ref()).expect("frame envelope");
                    (envelope["event"] == "im:interaction:frame").then(|| {
                        InteractionFrameV2::decode(
                            &serde_json::to_vec(&envelope["data"]).expect("frame data serializes"),
                        )
                        .expect("production Frame decodes")
                    })
                }
                _ => None,
            })
            .collect()
    }

    /// 返回当前 batch 的唯一 production Frame。
    fn only_frame(&self) -> InteractionFrameV2 {
        let frames = self.frames();
        assert_eq!(frames.len(), 1, "expected exactly one production Frame");
        frames.into_iter().next().expect("one frame exists")
    }
}

/// 构造 host ADR-007 HTTP 信封。
fn http_envelope(body: Value) -> Bytes {
    let raw = serde_json::to_vec(&body).expect("HTTP body serializes");
    Bytes::from(
        serde_json::to_vec(&json!({
            "status": 200,
            "headers": [],
            "body": base64_std(&raw),
        }))
        .expect("HTTP envelope serializes"),
    )
}

/// 以固定表实现测试侧标准 base64。
fn base64_std(bytes: &[u8]) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut output = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or_default() as u32;
        let b2 = chunk.get(2).copied().unwrap_or_default() as u32;
        let value = (b0 << 16) | (b1 << 8) | b2;
        output.push(TABLE[((value >> 18) & 0x3f) as usize] as char);
        output.push(TABLE[((value >> 12) & 0x3f) as usize] as char);
        output.push(if chunk.len() > 1 {
            TABLE[((value >> 6) & 0x3f) as usize] as char
        } else {
            '='
        });
        output.push(if chunk.len() > 2 {
            TABLE[(value & 0x3f) as usize] as char
        } else {
            '='
        });
    }
    output
}

/// 构造具有确定性身份和顺序的 render-ready 消息。
pub fn row(index: usize, channel_id: &str) -> Value {
    json!({
        "id": format!("server-{index:03}"),
        "temporaryId": format!("temporary-{index:03}"),
        "channelId": channel_id,
        "userId": "timeline-gate-viewer",
        "createAt": index as i64 + 1,
        "type": "TEXT",
        "message": format!("timeline message {index}"),
        "sendStatus": "sent",
        "revoke": false,
        "props": {},
        "actionCapabilities": {}
    })
}

/// 构造指定范围内的稳定消息集合。
pub fn rows(range: std::ops::Range<usize>, channel_id: &str) -> Vec<Value> {
    range.map(|index| row(index, channel_id)).collect()
}

/// 构造指定页长与双向边界的 Timeline 请求。
pub fn request(
    channel_id: &str,
    window_token: &str,
    page_size: u32,
    has_older: bool,
    has_newer: bool,
) -> TimelineWindowRequest {
    let mut request = TimelineWindowRequest::latest_with_window_token(channel_id, window_token);
    request.page_size = page_size;
    request.page.has_older = has_older;
    request.page.has_newer = has_newer;
    request.page.has_more = has_older || has_newer;
    request
}

/// 把请求改成包含目标消息的定位窗口。
#[allow(dead_code)]
pub fn locate(mut request: TimelineWindowRequest, target_index: usize) -> TimelineWindowRequest {
    let target = format!("server-{target_index:03}");
    request.anchor.mode = TimelineAnchorMode::Locate;
    request.anchor.message_id = Some(target.clone());
    request.target_message_id = Some(target);
    request
}

/// 使用正式 Frame composer 包装 producer mutation。
pub fn frame(transaction_id: &str, mutation: SurfaceMutation) -> InteractionFrameV2 {
    build_online_interaction_frame(transaction_id.to_owned(), 1, vec![mutation])
        .expect("formal timeline mutation must compose")
}

/// 从 Frame 中读取唯一 Timeline Delta 操作。
pub fn delta_operations(frame: &InteractionFrameV2) -> &[TimelineDeltaOperationVm] {
    let mutation = &frame.mutations[0].mutation;
    let TypedSurfaceMutation::TimelineDelta { operations } = &mutation.mutation else {
        panic!("expected TimelineDelta");
    };
    operations
}

/// 返回测试 crate 下的 canonical fixture 目录。
fn fixture_dir(directory: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(directory)
}

/// 生成稳定 pretty JSON bytes，并固定末尾换行。
fn canonical_bytes(value: &Value) -> Vec<u8> {
    let mut bytes = serde_json::to_vec_pretty(value).expect("fixture serializes");
    bytes.push(b'\n');
    bytes
}

/// 提取包含 revision、operation、key、range、bounds 与 watermark 的稳定语义投影。
fn semantic_bytes(frame: &InteractionFrameV2) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "schemaVersion": frame.schema_version,
        "mutations": frame.mutations.iter().map(|mutation| json!({
            "surface": mutation.mutation.surface,
            "surfaceSchemaVersion": mutation.surface_schema_version,
            "scope": mutation.mutation.scope,
            "revision": mutation.mutation.revision,
            "baseRevision": mutation.mutation.base_revision,
            "mutation": mutation.mutation.mutation,
        })).collect::<Vec<_>>(),
    }))
    .expect("semantic projection serializes")
}

/// 对比正式 Gate producer 输出，并仅在显式更新变量存在时写入 JSON 与 provenance。
pub fn assert_canonical_gate(
    gate_id: &str,
    test_class: &str,
    directory: &str,
    update_variable: &str,
    cases: &[(&str, InteractionFrameV2)],
) {
    let directory = fixture_dir(directory);
    let update = std::env::var_os(update_variable).is_some();
    let mut fixtures = Vec::with_capacity(cases.len());
    for (file_name, frame) in cases {
        frame.validate().expect("formal Gate frame remains valid");
        assert_eq!(frame.schema_version, 3);
        let value = serde_json::to_value(frame).expect("frame converts to JSON");
        let bytes = canonical_bytes(&value);
        if update {
            fs::create_dir_all(&directory).expect("fixture directory is writable");
            fs::write(directory.join(file_name), &bytes).expect("fixture export succeeds");
        }
        let checked_in = fs::read(directory.join(file_name)).unwrap_or_else(|error| {
            panic!(
                "{} missing ({error}); run {update_variable}=1 cargo test -p helix-im --test {test_class}",
                directory.join(file_name).display()
            )
        });
        assert_eq!(checked_in, bytes, "{file_name} drifted from producer");
        fixtures.push(json!({
            "fixtureId": format!("{gate_id}-{}", file_name.trim_end_matches(".frame-v3.json")),
            "fileName": file_name,
            "fixtureSha256": sha256_hex(&bytes),
            "semanticSha256": sha256_hex(&semantic_bytes(frame)),
            "schemaVersion": 3,
            "parameterizableFields": [
                "/transactionId",
                "/causationId",
                "/mutations/*/causationId",
                "/mutations/*/surfaceEpoch"
            ]
        }));
    }
    let mut semantic_coverage = vec![
        "operationKind",
        "entityKey",
        "orderedKeys",
        "rangeBounds",
        "latestWatermark",
        "revision",
        "baseRevision",
        "windowToken",
        "compositeCursor",
        "persistBarrier",
        "readback",
    ];
    if gate_id == "G-11a" {
        semantic_coverage.extend(["currentNavigationToken", "staleLocateSuppression"]);
    }
    let provenance = json!({
        "canonicalEligible": true,
        "evidencePlane": "helix-production-sans-io",
        "gateId": gate_id,
        "testClass": test_class,
        "producer": "ImModule::handle -> Effect::Http -> PortReply -> Effect::Persist -> PersistOk -> Effect::Persist(Get/Scan) -> PortReply -> Effect::Emit",
        "producerRevision": "312e331938054467f1de09d4a251fd47b12093b0",
        "readbackRequired": true,
        "pageSize": 20,
        "semanticCoverage": semantic_coverage,
        "fixtures": fixtures
    });
    let provenance_bytes = canonical_bytes(&provenance);
    if update {
        fs::write(directory.join("provenance.json"), &provenance_bytes)
            .expect("provenance export succeeds");
    }
    assert_eq!(
        fs::read(directory.join("provenance.json")).expect("provenance exists"),
        provenance_bytes,
        "{gate_id} provenance drifted"
    );
}
