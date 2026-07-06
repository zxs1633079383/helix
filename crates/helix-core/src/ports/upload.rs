//! FileUploader port — 本地文件上传抽象。

use crate::effect::{FileUploadRequest, FileUploadResponse};
use crate::error::PortError;
use crate::platform::{MaybeSend, MaybeSync};

#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
pub trait FileUploader: MaybeSend + MaybeSync + 'static {
    async fn upload(&self, req: FileUploadRequest) -> Result<FileUploadResponse, PortError>;
}
