mod durable;
mod multipart;
mod persistence;
mod plan;
mod request;
mod state;

use crate::state::TemporaryId;
use serde_json::Value;
use std::collections::{HashMap, HashSet, VecDeque};

pub use durable::{
    decode_scan as decode_durable_scan, delete_op as durable_delete_op,
    multipart_delete_ops as durable_multipart_delete_ops,
    multipart_part_upsert_op as durable_multipart_part_upsert_op,
    multipart_parts_delete_op as durable_multipart_parts_delete_op,
    multipart_parts_upsert_op as durable_multipart_parts_upsert_op,
    multipart_prepare_session_upsert_op as durable_multipart_prepare_session_upsert_op,
    multipart_session_upsert_op as durable_multipart_session_upsert_op,
    upsert_many as durable_upsert_many, upsert_op as durable_upsert_op,
};
pub use multipart::{
    multipart_complete_request, multipart_init_request, multipart_parts_request,
    multipart_sign_request, multipart_upload_request, parse_multipart_complete_reply,
    parse_multipart_init_reply, parse_multipart_parts_reply, parse_multipart_sign_reply,
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
    RichVideo { index: usize },
    File,
    TemplateImage,
}

impl UploadTarget {
    /// 返回 Java 签发票据与最终消息引用共同使用的稳定 bucket。
    pub fn bucket(&self) -> &'static str {
        match self {
            Self::RichImage { .. } => "picture",
            Self::RichVideo { .. } => "attachment",
            Self::File => "attachment",
            Self::TemplateImage => "picture",
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

/// Multipart applies only to the new Java protocol. The legacy PUT path remains
/// the behavior for files at or below 100 MiB.
pub const MULTIPART_THRESHOLD_BYTES: u64 = 100 * 1024 * 1024;
pub const MULTIPART_PART_SIZE_BYTES: u64 = 8 * 1024 * 1024;
pub const MAX_MEDIA_SIZE_BYTES: u64 = 1 << 30;

impl MediaInput {
    pub fn uses_multipart(&self) -> bool {
        self.size > MULTIPART_THRESHOLD_BYTES
    }

    pub fn part_count(&self) -> u32 {
        self.size.div_ceil(MULTIPART_PART_SIZE_BYTES) as u32
    }

    pub fn part_size(&self, part_number: u32) -> Option<u64> {
        if part_number == 0 || part_number > self.part_count() {
            return None;
        }
        let offset = (u64::from(part_number) - 1) * MULTIPART_PART_SIZE_BYTES;
        Some(
            self.size
                .saturating_sub(offset)
                .min(MULTIPART_PART_SIZE_BYTES),
        )
    }
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

/// A failed multipart batch remains in this draining state until every old
/// range PUT and already-issued checkpoint PersistAtomic have produced a
/// terminal reply. Retrying before that barrier could reuse a part number while
/// an old signed URL is still able to complete.
#[derive(Debug, Clone)]
pub(crate) struct MultipartDrain {
    pub pending: PendingMediaPrepare,
    pub retry_after_ms: Option<u64>,
    pub hop: &'static str,
    pub error: String,
}

#[derive(Debug, Clone)]
pub struct PendingMediaPut {
    pub temporary_id: TemporaryId,
    pub channel_id: crate::state::ChannelId,
    pub target: UploadTarget,
    pub input: MediaInput,
    pub prepared: PreparedMedia,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MultipartPart {
    pub part_number: u32,
    pub size: u64,
    pub etag: String,
}

#[derive(Debug, Clone)]
pub struct MultipartSession {
    pub temporary_id: TemporaryId,
    pub channel_id: crate::state::ChannelId,
    pub target: UploadTarget,
    pub input: MediaInput,
    pub client_upload_id: String,
    pub upload_id: String,
    pub file_id: String,
    pub bucket: String,
    pub object_key: String,
    pub uri: String,
    pub part_size: u64,
    pub status: String,
    pub parts: Vec<MultipartPart>,
}

#[derive(Debug, Clone)]
pub struct MultipartSignedPart {
    pub part_number: u32,
    pub url: String,
    pub headers: Vec<(String, String)>,
}

#[derive(Debug, Clone)]
pub struct PendingMultipartSign {
    pub session: MultipartSession,
    pub missing: Vec<u32>,
}

#[derive(Debug, Clone)]
pub struct PendingMultipartPut {
    pub session: MultipartSession,
    pub part: MultipartSignedPart,
    pub remaining: Vec<MultipartSignedPart>,
    /// The newly confirmed ETag whose single-row checkpoint must be persisted
    /// before the next signed part is emitted. `None` is used for the first
    /// in-memory UploadFile operation created from a sign response.
    pub checkpoint: Option<MultipartPart>,
}

/// Runtime-only batch state for one signed response. Durable facts remain in
/// `MultipartSession`/part rows; this queue only owns short-lived signed URLs
/// and at most three in-flight part numbers.
#[derive(Debug, Clone)]
pub(crate) struct MultipartBatch {
    pub session: MultipartSession,
    pub signed: VecDeque<MultipartSignedPart>,
    pub inflight: HashSet<u32>,
    /// Fixed-size presence index for confirmed remote parts. Progress ticks
    /// use it to avoid rescanning the confirmed list for every active range.
    pub confirmed_parts: [bool; 128],
    pub progress_bytes: HashMap<u32, u64>,
}

pub(crate) const MULTIPART_MAX_INFLIGHT: usize = 3;

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
    MultipartInit(PendingMediaPrepare),
    MultipartParts(MultipartSession),
    MultipartSign(PendingMultipartSign),
    MultipartPut(PendingMultipartPut),
    MultipartComplete(MultipartSession),
}

#[derive(Debug, Clone)]
pub enum FailedMediaOp {
    Prepare(PendingMediaPrepare),
    Complete(PendingMediaPut),
    Multipart(PendingMediaPrepare),
}

impl FailedMediaOp {
    pub fn temporary_id(&self) -> &TemporaryId {
        match self {
            Self::Prepare(pending) => &pending.temporary_id,
            Self::Complete(pending) => &pending.temporary_id,
            Self::Multipart(pending) => &pending.temporary_id,
        }
    }

    pub fn target(&self) -> &UploadTarget {
        match self {
            Self::Prepare(pending) => &pending.target,
            Self::Complete(pending) => &pending.target,
            Self::Multipart(pending) => &pending.target,
        }
    }

    pub fn channel_id(&self) -> crate::state::ChannelId {
        match self {
            Self::Prepare(pending) => pending.channel_id,
            Self::Complete(pending) => pending.channel_id,
            Self::Multipart(pending) => pending.channel_id,
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
