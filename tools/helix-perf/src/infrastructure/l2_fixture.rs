use futures_util::{SinkExt, StreamExt};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::oneshot;

pub async fn start_http_server() -> Result<(String, oneshot::Sender<()>, Arc<AtomicU64>), String> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| format!("PERF_FIXTURE_UNAVAILABLE: HTTP bind: {e}"))?;
    let address = listener.local_addr().map_err(|e| e.to_string())?;
    let (shutdown_tx, mut shutdown_rx) = oneshot::channel();
    let requests = Arc::new(AtomicU64::new(0));
    let requests_server = Arc::clone(&requests);
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = &mut shutdown_rx => break,
                accepted = listener.accept() => if let Ok((mut stream, _)) = accepted {
                    let requests = Arc::clone(&requests_server);
                    tokio::spawn(async move {
                        let mut request = [0_u8; 2048];
                        let _ = stream.read(&mut request).await;
                        requests.fetch_add(1, Ordering::Relaxed);
                        let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok").await;
                    });
                }
            }
        }
    });
    Ok((format!("http://{address}"), shutdown_tx, requests))
}

pub async fn start_ws_server() -> Result<(String, oneshot::Sender<()>, Arc<AtomicU64>), String> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| format!("PERF_FIXTURE_UNAVAILABLE: WS bind: {e}"))?;
    let address = listener.local_addr().map_err(|e| e.to_string())?;
    let (shutdown_tx, mut shutdown_rx) = oneshot::channel();
    let frames = Arc::new(AtomicU64::new(0));
    let frames_server = Arc::clone(&frames);
    tokio::spawn(async move {
        tokio::select! {
            _ = &mut shutdown_rx => {},
            accepted = listener.accept() => if let Ok((stream, _)) = accepted {
                if let Ok(mut socket) = tokio_tungstenite::accept_async(stream).await {
                    while let Some(Ok(message)) = socket.next().await {
                        frames_server.fetch_add(1, Ordering::Relaxed);
                        if socket.send(message).await.is_err() { break; }
                    }
                }
            }
        }
    });
    Ok((format!("ws://{address}"), shutdown_tx, frames))
}

pub fn cleanup_sqlite(path: &std::path::Path) -> Result<(), String> {
    for candidate in [
        path.to_path_buf(),
        path.with_extension("sqlite-wal"),
        path.with_extension("sqlite-shm"),
    ] {
        if candidate.exists() {
            std::fs::remove_file(&candidate).map_err(|e| format!("PERF_CLEANUP_NONZERO: {e}"))?;
        }
    }
    Ok(())
}

pub fn payload_size(bucket: &str) -> usize {
    match bucket {
        "256B" => 96,
        "16KiB" => 16_000,
        _ => 1_900,
    }
}

pub fn sqlite_residue(path: &std::path::Path) -> u64 {
    [
        path.to_path_buf(),
        path.with_extension("sqlite-wal"),
        path.with_extension("sqlite-shm"),
    ]
    .into_iter()
    .filter(|candidate| candidate.exists())
    .count() as u64
}
