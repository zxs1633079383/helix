//! Shared local-file uploader for native and FFI host drivers.

use std::sync::Arc;

use bytes::Bytes;
use futures_util::stream;
use reqwest::header;
use reqwest::{Client, Method};
use tokio::io::AsyncReadExt;

use helix_core::effect::{FileUploadProgress, FileUploadRequest, FileUploadResponse};
use helix_core::ports::{FileUploadProgressReporter, FileUploader};
use helix_core::PortError;

#[derive(Clone)]
pub struct SharedFileUploader {
    client: Client,
}

impl Default for SharedFileUploader {
    fn default() -> Self {
        Self::new(Client::new())
    }
}

impl SharedFileUploader {
    pub fn new(client: Client) -> Self {
        Self { client }
    }

    fn classify_error(e: reqwest::Error) -> PortError {
        let is_timeout = e.is_timeout();
        let is_network = e.is_connect() || e.is_request();
        // reqwest 错误默认可能携带完整 URL；先剥离，避免签名查询串进入 PortError/trace/log。
        let sanitized = e.without_url();
        if is_timeout {
            PortError::Transport(format!("timeout: {sanitized}"))
        } else if is_network {
            PortError::Transport(format!("network: {sanitized}"))
        } else {
            PortError::Http(sanitized.to_string())
        }
    }
}

#[async_trait::async_trait]
impl FileUploader for SharedFileUploader {
    async fn upload(&self, req: FileUploadRequest) -> Result<FileUploadResponse, PortError> {
        self.upload_with_progress(req, None).await
    }

    async fn upload_with_progress(
        &self,
        req: FileUploadRequest,
        progress: Option<Arc<dyn FileUploadProgressReporter>>,
    ) -> Result<FileUploadResponse, PortError> {
        let method = Method::from_bytes(req.method.as_bytes())
            .map_err(|e| PortError::Other(format!("invalid upload method: {e}")))?;
        if method != Method::PUT {
            return Err(PortError::Other(format!(
                "unsupported upload method: {}",
                req.method
            )));
        }

        let FileUploadRequest {
            local_path,
            object_key,
            urls,
            headers,
            content_type,
            size,
            ..
        } = req;
        let (upload_url, public_url) = urls.into_parts();

        let metadata = tokio::fs::metadata(&local_path)
            .await
            .map_err(|e| PortError::Other(format!("stat upload file failed: {e}")))?;
        let actual_size = metadata.len();
        if let Some(expected) = size {
            if actual_size != expected {
                return Err(PortError::Other(format!(
                    "upload file size mismatch: expected {expected}, got {actual_size}"
                )));
            }
        }
        let file = tokio::fs::File::open(&local_path)
            .await
            .map_err(|e| PortError::Other(format!("open upload file failed: {e}")))?;
        let body = reqwest::Body::wrap_stream(stream::unfold(
            (file, 0_u64, progress),
            move |(mut file, completed_bytes, progress)| async move {
                let mut chunk = vec![0_u8; 64 * 1024];
                match file.read(&mut chunk).await {
                    Ok(0) => None,
                    Ok(read) => {
                        chunk.truncate(read);
                        let completed_bytes = completed_bytes.saturating_add(read as u64);
                        if let Some(reporter) = progress.as_ref() {
                            reporter.report(FileUploadProgress {
                                completed_bytes,
                                total_bytes: actual_size,
                            });
                        }
                        Some((
                            Ok::<Bytes, std::io::Error>(Bytes::from(chunk)),
                            (file, completed_bytes, progress),
                        ))
                    }
                    Err(err) => Some((
                        Err::<Bytes, std::io::Error>(err),
                        (file, completed_bytes, progress),
                    )),
                }
            },
        ));

        let has_ticket_content_type = headers
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case(header::CONTENT_TYPE.as_str()));
        let mut builder = self.client.request(method, &upload_url);
        for (name, value) in headers {
            builder = builder.header(&name, value);
        }
        builder = builder.header(header::CONTENT_LENGTH, actual_size);
        if let Some(content_type) = content_type.filter(|_| !has_ticket_content_type) {
            builder = builder.header(header::CONTENT_TYPE, content_type);
        }

        let response = builder
            .body(body)
            .send()
            .await
            .map_err(Self::classify_error)?;
        let status = response.status();
        if !status.is_success() {
            return Err(PortError::Http(format!(
                "upload status {}",
                status.as_u16()
            )));
        }

        let etag = response
            .headers()
            .get(header::ETAG)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);

        Ok(FileUploadResponse {
            object_key,
            public_url,
            etag,
        })
    }
}
