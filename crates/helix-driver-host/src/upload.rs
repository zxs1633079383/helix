//! Shared local-file uploader for native and FFI host drivers.

use std::sync::Arc;
use std::time::{Duration, Instant};

use bytes::Bytes;
use futures_util::stream;
use reqwest::header;
use reqwest::{Client, Method};
use tokio::io::{AsyncReadExt, AsyncSeekExt};

use helix_core::effect::{FileUploadProgress, FileUploadRequest, FileUploadResponse};
use helix_core::ports::{FileUploadProgressReporter, FileUploader};
use helix_core::PortError;

/// Measured facts from one attempted PUT; no URL, headers, path, or body is exposed.
#[derive(Clone, Copy, Debug)]
pub struct UploadResponseObservation<'a> {
    pub method: &'static str,
    pub http_status: Option<u16>,
    pub object_key: &'a str,
    pub file_size: u64,
    pub duration_us: u64,
}

/// Driver-local synchronous observer; implementations enqueue rather than call a host callback.
pub trait UploadResponseObserver: Send + Sync {
    fn observe(&self, observation: UploadResponseObservation<'_>);
}

#[derive(Clone)]
pub struct SharedFileUploader {
    client: Client,
    timeout: Duration,
    part_timeout: Duration,
    part_idle_timeout: Duration,
    observer: Option<Arc<dyn UploadResponseObserver>>,
    request_guard: Option<crate::RequestAdmissionGuard>,
}

impl Default for SharedFileUploader {
    fn default() -> Self {
        Self::new(Client::new())
    }
}

impl SharedFileUploader {
    /// Opt-in host fact check; default adapters keep their previous network behavior.
    pub fn with_request_guard(mut self, guard: crate::RequestAdmissionGuard) -> Self {
        self.request_guard = Some(guard);
        self
    }

    /// 构造整文件 30 秒、分片 300 秒且空闲 60 秒的有界上传器。
    pub fn new(client: Client) -> Self {
        Self {
            client,
            timeout: Duration::from_secs(30),
            part_timeout: Duration::from_secs(300),
            part_idle_timeout: Duration::from_secs(60),
            observer: None,
            request_guard: None,
        }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// 区间上传使用独立的总时限与流读取/响应等待空闲预算，不延长旧整文件上传。
    pub fn with_part_timeouts(mut self, total: Duration, idle: Duration) -> Self {
        self.part_timeout = total;
        self.part_idle_timeout = idle;
        self
    }

    /// Add diagnostic observation without changing the upload result or core port.
    pub fn with_observer(mut self, observer: Arc<dyn UploadResponseObserver>) -> Self {
        self.observer = Some(observer);
        self
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

    /// 联网前校验文件与区间，按实际读取字节报告进度并等待上传响应。
    async fn upload_with_progress(
        &self,
        req: FileUploadRequest,
        progress: Option<Arc<dyn FileUploadProgressReporter>>,
    ) -> Result<FileUploadResponse, PortError> {
        if let Some(guard) = &self.request_guard { guard()?; }
        let range = req.range();
        if range.is_some() && (self.part_timeout.is_zero() || self.part_idle_timeout.is_zero()) {
            return Err(PortError::Other(
                "upload part timeouts must be positive".to_string(),
            ));
        }
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
        let transfer_size = match range {
            Some(range)
                if range.length > 0
                    && range
                        .offset
                        .checked_add(range.length)
                        .is_some_and(|end| end <= actual_size) =>
            {
                range.length
            }
            Some(_) => return Err(PortError::Other("upload file range is invalid".to_string())),
            None => actual_size,
        };
        let mut file = tokio::fs::File::open(&local_path)
            .await
            .map_err(|e| PortError::Other(format!("open upload file failed: {e}")))?;
        if let Some(range) = range {
            file.seek(std::io::SeekFrom::Start(range.offset))
                .await
                .map_err(|e| PortError::Other(format!("seek upload file failed: {e}")))?;
        }
        // A finite reader prevents a part request from leaking any suffix or full-file bytes.
        let file = file.take(transfer_size);
        let (activity_tx, mut activity_rx) = tokio::sync::watch::channel(());
        let body = reqwest::Body::wrap_stream(stream::unfold(
            (file, 0_u64, progress, activity_tx.clone()),
            move |(mut file, completed_bytes, progress, activity)| async move {
                let mut chunk = vec![0_u8; 64 * 1024];
                match file.read(&mut chunk).await {
                    Ok(0) => None,
                    Ok(read) => {
                        activity.send_replace(());
                        chunk.truncate(read);
                        let completed_bytes = completed_bytes.saturating_add(read as u64);
                        if let Some(reporter) = progress.as_ref() {
                            reporter.report(FileUploadProgress {
                                completed_bytes,
                                total_bytes: transfer_size,
                            });
                        }
                        Some((
                            Ok::<Bytes, std::io::Error>(Bytes::from(chunk)),
                            (file, completed_bytes, progress, activity),
                        ))
                    }
                    Err(err) => Some((
                        Err::<Bytes, std::io::Error>(err),
                        (file, completed_bytes, progress, activity),
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
        builder = builder.header(header::CONTENT_LENGTH, transfer_size);
        if let Some(content_type) = content_type.filter(|_| !has_ticket_content_type) {
            builder = builder.header(header::CONTENT_TYPE, content_type);
        }

        if let Some(guard) = &self.request_guard { guard()?; }
        let started = self.observer.as_ref().map(|_| Instant::now());
        let timeout = if range.is_some() {
            self.part_timeout
        } else {
            self.timeout
        };
        // The sender remains alive while awaiting acknowledgement, even after the body is read.
        let send = async {
            let send = builder.body(body).send();
            if range.is_none() {
                return send.await.map_err(Self::classify_error);
            }
            let idle = async {
                loop {
                    match tokio::time::timeout(self.part_idle_timeout, activity_rx.changed()).await
                    {
                        Ok(Ok(())) => {}
                        _ => {
                            return Err(PortError::Transport(format!(
                                "upload part idle timeout after {} ms",
                                self.part_idle_timeout.as_millis()
                            )))
                        }
                    }
                }
            };
            tokio::select! {
                result = send => result.map_err(Self::classify_error),
                result = idle => result,
            }
        };
        let response = tokio::time::timeout(timeout, send)
            .await
            .map_err(|_| PortError::Transport(format!("timeout after {} ms", timeout.as_millis())))
            .and_then(|response| response);
        drop(activity_tx);
        if let (Some(observer), Some(started)) = (&self.observer, started) {
            observer.observe(UploadResponseObservation {
                method: "PUT",
                http_status: response
                    .as_ref()
                    .ok()
                    .map(|response| response.status().as_u16()),
                object_key: &object_key,
                file_size: transfer_size,
                duration_us: started.elapsed().as_micros().min(u64::MAX as u128) as u64,
            });
        }
        let response = response?;
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

        // A part is durable only with its OSS acknowledgement; status alone cannot resume it.
        if range.is_some() && etag.as_deref().is_none_or(|value| value.trim().is_empty()) {
            return Err(PortError::Http(
                "upload part response missing ETag".to_string(),
            ));
        }

        Ok(FileUploadResponse {
            object_key,
            public_url,
            etag,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use helix_core::effect::FileUploadUrls;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Mutex;
    use tokio::io::AsyncWriteExt;
    use tokio::net::TcpListener;
    use tokio::sync::oneshot;

    #[derive(Debug)]
    struct RecordedResponse {
        method: &'static str,
        status: Option<u16>,
        object_key: String,
        file_size: u64,
        duration_us: u64,
    }

    #[derive(Default)]
    struct Recorder(Mutex<Vec<RecordedResponse>>);

    impl UploadResponseObserver for Recorder {
        fn observe(&self, fact: UploadResponseObservation<'_>) {
            self.0.lock().unwrap().push(RecordedResponse {
                method: fact.method,
                status: fact.http_status,
                object_key: fact.object_key.to_owned(),
                file_size: fact.file_size,
                duration_us: fact.duration_us,
            });
        }
    }

    // Exercise the real streaming uploader, rather than a mocked observer call.
    async fn actual_put(status: Option<u16>) -> Result<FileUploadResponse, PortError> {
        let payload: Vec<u8> = (0..130_037).map(|i| (i % 251) as u8).collect();
        let expected_payload = payload.clone();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (request_tx, request_rx) = oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut chunk = [0; 4096];
            let header_end = loop {
                let read = socket.read(&mut chunk).await.unwrap();
                assert_ne!(read, 0, "PUT must include headers and full body");
                request.extend_from_slice(&chunk[..read]);
                if let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                    break end + 4;
                }
            };
            let headers = std::str::from_utf8(&request[..header_end]).unwrap();
            assert!(headers.starts_with("PUT /object?signature=synthetic-secret HTTP/1.1"));
            let content_length: usize = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse().unwrap())
                })
                .unwrap();
            assert_eq!(content_length, expected_payload.len());
            while request.len() < header_end + content_length {
                let read = socket.read(&mut chunk).await.unwrap();
                assert_ne!(read, 0, "PUT body must match actual metadata length");
                request.extend_from_slice(&chunk[..read]);
            }
            assert_eq!(&request[header_end..], expected_payload);
            request_tx.send(()).unwrap();
            if let Some(status) = status {
                socket.write_all(format!(
                    "HTTP/1.1 {status} Test\r\nETag: measured-etag\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                ).as_bytes()).await.unwrap();
            } else {
                std::future::pending::<()>().await;
            }
        });
        static FIXTURE_SEQUENCE: AtomicU64 = AtomicU64::new(0);
        let fixture_id = FIXTURE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let local_path = std::env::temp_dir().join(format!(
            "helix-upload-diagnostic-{}-{fixture_id}.bin",
            std::process::id()
        ));
        tokio::fs::write(&local_path, &payload).await.unwrap();
        let recorder = Arc::new(Recorder::default());
        let client = Client::builder().no_proxy().build().unwrap();
        let upload_started = Instant::now();
        let result = SharedFileUploader::new(client)
            .with_timeout(Duration::from_millis(150))
            .with_observer(recorder.clone())
            .upload(FileUploadRequest {
                local_path: local_path.to_string_lossy().into_owned(),
                object_key: "actual/request-key.bin".to_owned(),
                method: "PUT".to_owned(),
                urls: FileUploadUrls::new(
                    format!("http://{address}/object?signature=synthetic-secret"),
                    "https://cdn.example.test/public/object.bin".to_owned(),
                )
                .unwrap(),
                headers: vec![("Cookie".to_owned(), "synthetic-private".to_owned())],
                content_type: Some("application/octet-stream".to_owned()),
                size: None, // Observation must use metadata, not a requested-size assumption.
            })
            .await;
        let upload_duration_us = upload_started.elapsed().as_micros();
        tokio::time::timeout(Duration::from_secs(1), request_rx)
            .await
            .unwrap()
            .unwrap();
        if status.is_some() {
            server.await.unwrap();
        } else {
            server.abort();
            assert!(server.await.unwrap_err().is_cancelled());
        }
        tokio::fs::remove_file(local_path).await.unwrap();
        let facts = recorder.0.lock().unwrap();
        assert_eq!(facts.len(), 1, "one observation per attempted PUT");
        assert_eq!(facts[0].method, "PUT");
        assert_eq!(facts[0].status, status);
        assert_eq!(facts[0].object_key, "actual/request-key.bin");
        assert_eq!(facts[0].file_size, payload.len() as u64);
        assert!(u128::from(facts[0].duration_us) <= upload_duration_us);
        result
    }

    #[tokio::test]
    async fn upload_diagnostic_actual_put_2xx_keeps_success_result() {
        let response = actual_put(Some(201)).await.unwrap();
        assert_eq!(response.object_key, "actual/request-key.bin");
        assert_eq!(
            response.public_url,
            "https://cdn.example.test/public/object.bin"
        );
        assert_eq!(response.etag.as_deref(), Some("measured-etag"));
    }

    #[tokio::test]
    async fn upload_diagnostic_actual_put_5xx_keeps_http_error() {
        assert!(matches!(actual_put(Some(503)).await,
            Err(PortError::Http(message)) if message == "upload status 503"));
    }

    #[tokio::test]
    async fn upload_diagnostic_actual_put_timeout_has_no_invented_status() {
        assert!(matches!(actual_put(None).await,
            Err(PortError::Transport(message)) if message == "timeout after 150 ms"));
    }
}
