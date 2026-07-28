mod durable;
mod persistence;
mod plan;
mod request;
mod state;

use crate::state::TemporaryId;
use serde_json::Value;

pub use durable::{
    decode_scan as decode_durable_scan, delete_op as durable_delete_op,
    upsert_many as durable_upsert_many, upsert_op as durable_upsert_op,
};
pub use persistence::{media_send_state_persist_op, props_persist_op, sanitized_for_persistence};
pub use plan::build_upload_plan;
pub use request::{
    client_upload_id, file_upload_request, media_upload_request, parse_prepare_reply,
    prepare_upload_request, validate_java_api_base_url,
};
pub use state::{
    mark_media_complete, mark_media_stage, mark_upload_failed, mark_upload_success,
    parse_upload_success_reply,
};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum UploadTarget {
    RichImage { index: usize },
    File,
}

impl UploadTarget {
    /// 返回 Java 签发票据与最终消息引用共同使用的稳定 bucket。
    pub fn bucket(&self) -> &'static str {
        match self {
            Self::RichImage { .. } => "picture",
            Self::File => "attachment",
        }
    }
}

#[derive(Debug, Clone)]
pub struct UploadPlan {
    pub props: Value,
    pub targets: Vec<UploadTarget>,
    pub media: Vec<PlannedMedia>,
}

#[derive(Debug, Clone)]
pub struct PlannedMedia {
    pub target: UploadTarget,
    pub input: MediaInput,
}

#[derive(Debug, Clone)]
pub struct MediaInput {
    pub local_path: String,
    pub file_name: String,
    pub content_type: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Debug, Clone)]
pub struct PreparedMedia {
    pub file_id: String,
    pub bucket: String,
    pub method: String,
    pub url: String,
    pub uri: String,
    pub object_key: String,
    pub headers: Vec<(String, String)>,
}

#[derive(Debug, Clone)]
pub struct PendingMediaPrepare {
    pub temporary_id: TemporaryId,
    pub channel_id: crate::state::ChannelId,
    pub target: UploadTarget,
    pub input: MediaInput,
}

#[derive(Debug, Clone)]
pub struct PendingMediaPut {
    pub temporary_id: TemporaryId,
    pub channel_id: crate::state::ChannelId,
    pub target: UploadTarget,
    pub input: MediaInput,
    pub prepared: PreparedMedia,
}

#[derive(Debug, Clone)]
pub struct PendingMediaCompletionCommit {
    pub pending: PendingMediaPut,
    pub props_after: Value,
}

#[derive(Debug, Clone)]
pub struct PendingMediaRetryReset {
    pub temporary_id: TemporaryId,
    pub channel_id: crate::state::ChannelId,
    pub body_after: Value,
    pub timeline_readback: crate::pending_send::TimelineReadbackContext,
    pub operations: Vec<FailedMediaOp>,
}

#[derive(Debug, Clone)]
pub enum PendingMediaOp {
    Prepare(PendingMediaPrepare),
    Put(PendingMediaPut),
    Complete(PendingMediaPut),
}

#[derive(Debug, Clone)]
pub enum FailedMediaOp {
    Prepare(PendingMediaPrepare),
    Complete(PendingMediaPut),
}

impl FailedMediaOp {
    pub fn temporary_id(&self) -> &TemporaryId {
        match self {
            Self::Prepare(pending) => &pending.temporary_id,
            Self::Complete(pending) => &pending.temporary_id,
        }
    }

    pub fn target(&self) -> &UploadTarget {
        match self {
            Self::Prepare(pending) => &pending.target,
            Self::Complete(pending) => &pending.target,
        }
    }

    pub fn channel_id(&self) -> crate::state::ChannelId {
        match self {
            Self::Prepare(pending) => pending.channel_id,
            Self::Complete(pending) => pending.channel_id,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PendingUpload {
    pub temporary_id: TemporaryId,
    pub target: UploadTarget,
}

#[derive(Debug, Clone)]
pub struct UploadSuccess {
    pub object_key: String,
    pub public_url: String,
    pub etag: Option<String>,
}
