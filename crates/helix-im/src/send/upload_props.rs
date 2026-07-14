mod persistence;
mod plan;
mod request;
mod state;

use crate::state::TemporaryId;
use serde_json::Value;

pub use persistence::{props_persist_op, sanitized_for_persistence};
pub use plan::build_upload_plan;
pub use request::file_upload_request;
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
