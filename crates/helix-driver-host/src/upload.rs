//! Shared local-file uploader for native and FFI host drivers.

use reqwest::header;
use reqwest::{Client, Method};

use helix_core::effect::{FileUploadRequest, FileUploadResponse};
use helix_core::ports::FileUploader;
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

    fn classify_error(e: &reqwest::Error) -> PortError {
        if e.is_timeout() {
            PortError::Transport(format!("timeout: {e}"))
        } else if e.is_connect() || e.is_request() {
            PortError::Transport(format!("network: {e}"))
        } else {
            PortError::Http(e.to_string())
        }
    }
}

#[async_trait::async_trait]
impl FileUploader for SharedFileUploader {
    async fn upload(&self, req: FileUploadRequest) -> Result<FileUploadResponse, PortError> {
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
            url,
            headers,
            content_type,
            size,
            ..
        } = req;

        let bytes = tokio::task::spawn_blocking({
            let local_path = local_path.clone();
            move || std::fs::read(local_path)
        })
        .await
        .map_err(|e| PortError::Other(format!("read upload file join failed: {e}")))?
        .map_err(|e| PortError::Other(format!("read upload file failed: {e}")))?;

        if let Some(expected) = size {
            if bytes.len() as u64 != expected {
                return Err(PortError::Other(format!(
                    "upload file size mismatch: expected {expected}, got {}",
                    bytes.len()
                )));
            }
        }

        let mut builder = self.client.request(method, &url);
        for (name, value) in headers {
            builder = builder.header(&name, value);
        }
        if let Some(content_type) = content_type {
            builder = builder.header(header::CONTENT_TYPE, content_type);
        }

        let response = builder
            .body(bytes)
            .send()
            .await
            .map_err(|e| Self::classify_error(&e))?;
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
            url: Some(url),
            etag,
        })
    }
}
