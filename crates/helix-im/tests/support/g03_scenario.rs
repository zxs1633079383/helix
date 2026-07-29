use helix_im::interaction_frame::{
    InteractionFrameV2, InteractionRuntimeVm, RuntimeConnectionState, RuntimeRecoveryState,
    RuntimeSyncState, SurfaceMutationV2, TypedSurfaceMutation, INTERACTION_FRAME_SCHEMA_VERSION,
    TIMELINE_SURFACE_SCHEMA_VERSION,
};
use helix_im::projector::timeline::{TimelineProjector, TimelineWindowRequest};
use serde_json::{json, Value};

pub const CHANNEL_ID: &str = "chg03media00000000000000001";
pub const ACTOR_ID: &str = "g03-wdio-actor";
pub const CAPTION: &str = "G03 图文投影";
pub const IMAGE_POST_ID: &str = "g03-image-post-1";
pub const FILE_POST_ID: &str = "g03-file-post-1";
pub const CREATED_AT: i64 = 1_700_000_300_003;

pub struct G03FrameScenario {
    pub image_sending: InteractionFrameV2,
    pub image_sent: InteractionFrameV2,
    pub file_uploading: InteractionFrameV2,
    pub file_sent: InteractionFrameV2,
}

pub struct G03ScenarioRunner;

impl G03ScenarioRunner {
    pub fn run() -> G03FrameScenario {
        G03FrameScenario {
            image_sending: frame("image-sending", image_row("sending")),
            image_sent: frame("image-sent", image_row("sent")),
            file_uploading: frame("file-uploading", file_row("sending", true)),
            file_sent: frame("file-sent", file_row("sent", false)),
        }
    }
}

/// Wraps one projected media mutation in the canonical Timeline V3 surface envelope.
fn frame(case: &str, row: Value) -> InteractionFrameV2 {
    let mut projector = TimelineProjector::default();
    let mutation = projector
        .snapshot_from_render_ready(TimelineWindowRequest::latest(CHANNEL_ID), &[row])
        .expect("G03 render-ready row projects through the canonical timeline Surface");
    let frame = InteractionFrameV2 {
        schema_version: INTERACTION_FRAME_SCHEMA_VERSION,
        transaction_id: format!("g03-{case}-transaction"),
        causation_id: Some(format!("g03-{case}-request")),
        runtime: InteractionRuntimeVm {
            connection: RuntimeConnectionState::Online,
            initial_sync: RuntimeSyncState::Ready,
            recovery: RuntimeRecoveryState::Idle,
        },
        mutations: vec![SurfaceMutationV2 {
            mutation,
            surface_epoch: 1,
            surface_schema_version: Some(TIMELINE_SURFACE_SCHEMA_VERSION),
        }],
        directives: vec![],
    };
    frame
        .validate()
        .expect("G03 producer emits one valid InteractionFrame V2");
    frame
}

fn image_row(send_status: &str) -> Value {
    json!({
        "msgId": IMAGE_POST_ID,
        "channelId": CHANNEL_ID,
        "eventSeq": 31,
        "userId": ACTOR_ID,
        "createAt": CREATED_AT,
        "text": CAPTION,
        "type": "IMAGE",
        "sendStatus": send_status,
        "props": {
            "imagePosition": "bottom",
            "files": [
                {
                    "id": "g03-image-a",
                    "name": "g03-a.png",
                    "size": 68,
                    "contentType": "image/png",
                    "bucket": "picture",
                    "uri": "media/g03-image-a"
                },
                {
                    "id": "g03-image-b",
                    "name": "g03-b.png",
                    "size": 70,
                    "contentType": "image/png",
                    "bucket": "picture",
                    "uri": "media/g03-image-b"
                }
            ]
        },
        "actionCapabilities": {}
    })
}

fn file_row(send_status: &str, uploading: bool) -> Value {
    json!({
        "msgId": FILE_POST_ID,
        "channelId": CHANNEL_ID,
        "eventSeq": 32,
        "userId": ACTOR_ID,
        "createAt": CREATED_AT + 1,
        "text": "G03 文件投影",
        "type": "FILE",
        "sendStatus": send_status,
        "progressPercent": if uploading { 45 } else { 100 },
        "props": {
            "file": {
                "id": "g03-file-a",
                "name": "g03-document.txt",
                "size": 33,
                "contentType": "text/plain",
                "bucket": "attachment",
                "uri": "media/g03-file-a",
                "upload": {
                    "status": if uploading { "uploading" } else { "completed" }
                }
            }
        },
        "actionCapabilities": {}
    })
}

pub fn assert_g03_frame_contract(frame: &InteractionFrameV2, expected: &str) {
    let message = frame
        .mutations
        .iter()
        .find_map(|mutation| match &mutation.mutation.mutation {
            TypedSurfaceMutation::TimelineSnapshot { view } => view.items.first(),
            _ => None,
        })
        .expect("G03 fixture contains one typed timeline message");
    let content = serde_json::to_value(&message.content).expect("content serializes");
    let delivery = serde_json::to_value(&message.delivery).expect("delivery serializes");
    match expected {
        "image-sending" | "image-sent" => {
            assert_eq!(content["kind"], "media");
            assert_eq!(content["mediaType"], "image");
            assert_eq!(content["caption"], CAPTION);
            assert_eq!(content["imagePosition"], "bottom");
            assert_eq!(content["items"].as_array().map(Vec::len), Some(2));
            assert_eq!(content["items"][0]["fileId"], "g03-image-a");
            assert_eq!(content["items"][1]["fileId"], "g03-image-b");
            assert_eq!(content["items"][0]["bucket"], "picture");
            assert_eq!(content["items"][0]["uri"], "media/g03-image-a");
            assert_ne!(delivery["kind"], "uploading");
        }
        "file-uploading" => {
            assert_eq!(content["mediaType"], "file");
            assert_eq!(content["items"][0]["fileId"], "g03-file-a");
            assert_eq!(content["items"][0]["bucket"], "attachment");
            assert_eq!(content["items"][0]["uri"], "media/g03-file-a");
            assert_eq!(
                delivery,
                json!({"kind": "uploading", "progressPercent": 45})
            );
        }
        "file-sent" => {
            assert_eq!(content["mediaType"], "file");
            assert_eq!(content["items"][0]["fileName"], "g03-document.txt");
            assert_eq!(content["items"][0]["bucket"], "attachment");
            assert_eq!(content["items"][0]["uri"], "media/g03-file-a");
            assert_eq!(delivery["kind"], "sent");
        }
        other => panic!("unknown G03 fixture case {other}"),
    }
}
