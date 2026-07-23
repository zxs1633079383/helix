use std::time::{SystemTime, UNIX_EPOCH};

use helix_core::effect::{FileUploadRequest, FileUploadUrls};
use helix_core::ports::FileUploader;
use helix_driver_host::SharedFileUploader;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

#[tokio::test]
async fn put_uses_signed_upload_url_but_response_only_contains_public_url() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind local upload server");
    let address = listener.local_addr().expect("local address");
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept upload");
        let mut request = Vec::new();
        let mut chunk = [0_u8; 4096];
        loop {
            let read = stream.read(&mut chunk).await.expect("read upload request");
            if read == 0 {
                break;
            }
            request.extend_from_slice(&chunk[..read]);
            if request.windows(4).any(|window| window == b"\r\n\r\n")
                && request.ends_with(b"payload")
            {
                break;
            }
        }
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nETag: test-etag\r\nContent-Length: 0\r\n\r\n")
            .await
            .expect("write upload response");
        request
    });

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after epoch")
        .as_nanos();
    let local_path = std::env::temp_dir().join(format!("helix-upload-{nonce}.txt"));
    tokio::fs::write(&local_path, b"payload")
        .await
        .expect("write upload fixture");
    let signed_upload_url =
        format!("http://{address}/put/object?X-Amz-Signature=write-secret&X-Amz-Expires=60");
    let public_url = "https://cdn.example.com/im/object.txt";

    let client = reqwest::Client::builder()
        .no_proxy()
        .build()
        .expect("build isolated HTTP client");
    let response = SharedFileUploader::new(client)
        .upload(FileUploadRequest {
            local_path: local_path.to_string_lossy().into_owned(),
            object_key: "im/object.txt".to_string(),
            method: "PUT".to_string(),
            urls: FileUploadUrls::new(signed_upload_url.clone(), public_url.to_string())
                .expect("distinct upload urls"),
            headers: vec![
                ("Content-Type".to_string(), "text/plain".to_string()),
                ("x-oss-forbid-overwrite".to_string(), "true".to_string()),
            ],
            content_type: Some("text/plain".to_string()),
            size: Some(7),
        })
        .await
        .expect("upload succeeds");

    let request = server.await.expect("local upload server joins");
    let request_text = String::from_utf8_lossy(&request);
    assert!(
        request_text
            .starts_with("PUT /put/object?X-Amz-Signature=write-secret&X-Amz-Expires=60 HTTP/1.1"),
        "request must target the signed PUT URL: {request_text}"
    );
    assert_eq!(response.object_key, "im/object.txt");
    assert_eq!(response.public_url, public_url);
    assert_eq!(response.etag.as_deref(), Some("test-etag"));
    assert!(!response.public_url.contains("X-Amz-Signature"));
    assert_ne!(response.public_url, signed_upload_url);
    assert_eq!(
        request_text
            .lines()
            .filter(|line| line.to_ascii_lowercase().starts_with("content-type:"))
            .count(),
        1,
        "ticket Content-Type is authoritative and must not be appended twice: {request_text}"
    );
    assert!(
        request_text
            .to_ascii_lowercase()
            .contains("x-oss-forbid-overwrite: true"),
        "signed OSS headers must be forwarded unchanged: {request_text}"
    );

    tokio::fs::remove_file(local_path)
        .await
        .expect("remove upload fixture");
}
