mod persistence;
mod plan;
mod request;
mod state;

use crate::state::TemporaryId;
use serde_json::Value;

pub use persistence::{props_persist_op, sanitized_for_persistence};
pub use plan::build_upload_plan;
pub use request::{
    file_upload_request, media_upload_request, parse_prepare_reply, prepare_upload_request,
};
pub use state::{mark_upload_failed, mark_upload_success, parse_upload_success_reply};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum UploadTarget {
    RichImage { index: usize },
    File,
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
    pub upload_id: String,
    pub file_id: String,
    pub method: String,
    pub url: String,
    pub public_url: String,
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
pub enum PendingMediaOp {
    Prepare(PendingMediaPrepare),
    Put(PendingMediaPut),
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
