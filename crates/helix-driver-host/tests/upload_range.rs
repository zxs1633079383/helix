//! Real local HTTP seam for bounded file-range PUTs; no OSS acceptance claim.
use helix_core::effect::{FileUploadRange, FileUploadRequest, FileUploadUrls};
use helix_core::ports::FileUploader;
use helix_core::PortError;
use helix_driver_host::SharedFileUploader;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// A sparse 1 GiB file must upload only the selected final bytes, never its prefix.
#[tokio::test]
async fn sparse_gib_file_put_reads_only_selected_tail() {
    let path = std::env::temp_dir().join(format!("helix-range-{}.bin", std::process::id()));
    let mut file = tokio::fs::File::create(&path).await.unwrap();
    file.set_len(1_073_741_824).await.unwrap();
    tokio::io::AsyncSeekExt::seek(&mut file, std::io::SeekFrom::Start(1_073_741_821))
        .await
        .unwrap();
    file.write_all(b"xyz").await.unwrap();
    drop(file);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        let mut buffer = [0; 2048];
        let header_end = loop {
            let n = socket.read(&mut buffer).await.unwrap();
            assert!(n > 0);
            request.extend_from_slice(&buffer[..n]);
            if let Some(pos) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                break pos + 4;
            }
        };
        let headers = String::from_utf8_lossy(&request[..header_end]);
        assert!(headers.to_lowercase().contains("content-length: 3\r\n"));
        while request.len() < header_end + 3 {
            let n = socket.read(&mut buffer).await.unwrap();
            assert!(n > 0);
            request.extend_from_slice(&buffer[..n]);
        }
        assert_eq!(&request[header_end..], b"xyz");
        socket.write_all(b"HTTP/1.1 200 OK\r\nETag: tail-etag\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await.unwrap();
    });
    let urls = FileUploadUrls::new(format!("http://{addr}/part"), "media/file".into())
        .unwrap()
        .with_range(FileUploadRange {
            offset: 1_073_741_821,
            length: 3,
        });
    let result = SharedFileUploader::default()
        .upload(FileUploadRequest {
            local_path: path.to_string_lossy().into_owned(),
            object_key: "media/file".into(),
            method: "PUT".into(),
            urls,
            headers: vec![],
            content_type: None,
            size: Some(1_073_741_824),
        })
        .await;
    let peer_result = server.await;
    tokio::fs::remove_file(path).await.unwrap();
    peer_result.unwrap();
    assert_eq!(result.unwrap().etag.as_deref(), Some("tail-etag"));
}

/// Exercise missing acknowledgement and timeout against an actual local HTTP peer.
async fn part_attempt_with_budgets(
    etag: bool,
    stall_response: bool,
    total_budget: std::time::Duration,
    idle_budget: std::time::Duration,
) -> Result<helix_core::effect::FileUploadResponse, PortError> {
    static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let id = SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("helix-part-{}-{id}.bin", std::process::id()));
    tokio::fs::write(&path, b"abcdef").await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut buffer = [0; 2048];
        let mut bytes = Vec::new();
        loop {
            let read = socket.read(&mut buffer).await.unwrap();
            assert!(read > 0);
            bytes.extend_from_slice(&buffer[..read]);
            if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                if bytes.len() >= end + 7 {
                    assert_eq!(&bytes[end + 4..], b"bcd");
                    break;
                }
            }
        }
        if stall_response {
            std::future::pending::<()>().await;
        }
        let header = if etag { "ETag: valid-part\r\n" } else { "" };
        socket
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\n{header}Content-Length: 0\r\nConnection: close\r\n\r\n"
                )
                .as_bytes(),
            )
            .await
            .unwrap();
    });
    let request = FileUploadRequest {
        local_path: path.to_string_lossy().into_owned(),
        object_key: "media/file".into(),
        method: "PUT".into(),
        urls: FileUploadUrls::new(format!("http://{addr}/part"), "media/file".into())
            .unwrap()
            .with_range(FileUploadRange {
                offset: 1,
                length: 3,
            }),
        headers: vec![],
        content_type: None,
        size: Some(6),
    };
    let result = SharedFileUploader::default()
        .with_timeout(std::time::Duration::from_millis(1))
        .with_part_timeouts(total_budget, idle_budget)
        .upload(request)
        .await;
    server.abort();
    let _ = server.await;
    tokio::fs::remove_file(path).await.unwrap();
    result
}

/// A successful status without a part ETag cannot be checkpointed as complete.
#[tokio::test]
async fn part_put_requires_a_nonempty_etag() {
    assert!(matches!(part_attempt(false, false).await,
        Err(PortError::Http(message)) if message.contains("ETag")));
}

/// A part waiting for an acknowledgement stops on its idle budget and can be reconciled later.
#[tokio::test]
async fn stalled_part_response_hits_idle_timeout() {
    assert!(matches!(part_attempt(true, true).await,
        Err(PortError::Transport(message)) if message.contains("idle timeout")));
}

/// Overflow, empty and out-of-file slices fail before any network request is sent.
#[tokio::test]
async fn invalid_part_ranges_fail_closed_before_network() {
    let path = std::env::temp_dir().join(format!("helix-invalid-range-{}.bin", std::process::id()));
    tokio::fs::write(&path, b"abc").await.unwrap();
    for range in [
        FileUploadRange {
            offset: 0,
            length: 0,
        },
        FileUploadRange {
            offset: 2,
            length: 2,
        },
        FileUploadRange {
            offset: u64::MAX,
            length: 1,
        },
    ] {
        let result = SharedFileUploader::default()
            .upload(FileUploadRequest {
                local_path: path.to_string_lossy().into_owned(),
                object_key: "media/file".into(),
                method: "PUT".into(),
                headers: vec![],
                content_type: None,
                size: Some(3),
                urls: FileUploadUrls::new("http://127.0.0.1:9/part".into(), "media/file".into())
                    .unwrap()
                    .with_range(range),
            })
            .await;
        assert!(matches!(result, Err(PortError::Other(message)) if message.contains("range")));
    }
    tokio::fs::remove_file(path).await.unwrap();
}

/// Normal part tests use bounded budgets independently of the legacy one millisecond timeout.
async fn part_attempt(
    etag: bool,
    stall_response: bool,
) -> Result<helix_core::effect::FileUploadResponse, PortError> {
    part_attempt_with_budgets(
        etag,
        stall_response,
        std::time::Duration::from_millis(200),
        std::time::Duration::from_millis(20),
    )
    .await
}

/// A part's total limit still terminates an attempt even if the idle limit is larger.
#[tokio::test]
async fn part_total_timeout_is_independent_of_whole_file_timeout() {
    let result = part_attempt_with_budgets(
        true,
        true,
        std::time::Duration::from_millis(15),
        std::time::Duration::from_secs(1),
    )
    .await;
    assert!(
        matches!(result, Err(PortError::Transport(message)) if message == "timeout after 15 ms")
    );
}

/// Record source reads, independently of the peer's received bytes.
struct ReadFacts(std::sync::atomic::AtomicU64);
impl helix_core::ports::FileUploadProgressReporter for ReadFacts {
    fn report(&self, progress: helix_core::effect::FileUploadProgress) {
        self.0.store(
            progress.completed_bytes,
            std::sync::atomic::Ordering::SeqCst,
        );
    }
}

/// Timing out a stalled receiver must drop the body stream, not continue it in a hidden task.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn timed_out_part_does_not_continue_source_reads_after_peer_unblocks() {
    use std::sync::atomic::Ordering;
    let path = std::env::temp_dir().join(format!("helix-cancel-range-{}.bin", std::process::id()));
    let file = tokio::fs::File::create(&path).await.unwrap();
    file.set_len(1_073_741_824).await.unwrap();
    drop(file);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (drain_tx, drain_rx) = tokio::sync::oneshot::channel();
    let peer = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        drain_rx.await.unwrap();
        let mut buffer = [0; 64 * 1024];
        loop {
            match tokio::time::timeout(std::time::Duration::from_secs(2), socket.read(&mut buffer))
                .await
            {
                Ok(Ok(0)) | Ok(Err(_)) => break,
                Ok(Ok(_)) => {}
                Err(_) => panic!("cancelled request must release its peer connection"),
            }
        }
    });
    let reads = std::sync::Arc::new(ReadFacts(std::sync::atomic::AtomicU64::new(0)));
    let result = SharedFileUploader::default()
        .with_part_timeouts(
            std::time::Duration::from_millis(500),
            std::time::Duration::from_millis(100),
        )
        .upload_with_progress(
            FileUploadRequest {
                local_path: path.to_string_lossy().into_owned(),
                object_key: "media/cancel".into(),
                method: "PUT".into(),
                headers: vec![],
                content_type: None,
                size: Some(1_073_741_824),
                urls: FileUploadUrls::new(format!("http://{addr}/part"), "media/cancel".into())
                    .unwrap()
                    .with_range(FileUploadRange {
                        offset: 0,
                        length: 1_073_741_824,
                    }),
            },
            Some(reads.clone()),
        )
        .await;
    let at_failure = reads.0.load(Ordering::SeqCst);
    let _ = drain_tx.send(());
    let peer_result = peer.await;
    tokio::fs::remove_file(path).await.unwrap();
    peer_result.unwrap();
    assert!(matches!(result, Err(PortError::Transport(_))));
    assert_eq!(
        reads.0.load(Ordering::SeqCst),
        at_failure,
        "dropped request cannot read more source bytes"
    );
    assert!(
        at_failure < 1_073_741_824,
        "stalled peer cannot consume a whole GiB before cancellation"
    );
}
