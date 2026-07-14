use futures_util::{SinkExt, StreamExt};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::oneshot;

pub async fn start_http_server() -> Result<(String, oneshot::Sender<()>), String> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| format!("PERF_FIXTURE_UNAVAILABLE: HTTP bind: {e}"))?;
    let address = listener.local_addr().map_err(|e| e.to_string())?;
    let (shutdown_tx, mut shutdown_rx) = oneshot::channel();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = &mut shutdown_rx => break,
                accepted = listener.accept() => if let Ok((mut stream, _)) = accepted {
                    tokio::spawn(async move {
                        let mut request = [0_u8; 2048];
                        let _ = stream.read(&mut request).await;
                        let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok").await;
                    });
                }
            }
        }
    });
    Ok((format!("http://{address}/health"), shutdown_tx))
}

pub async fn start_ws_server() -> Result<(String, oneshot::Sender<()>), String> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| format!("PERF_FIXTURE_UNAVAILABLE: WS bind: {e}"))?;
    let address = listener.local_addr().map_err(|e| e.to_string())?;
    let (shutdown_tx, mut shutdown_rx) = oneshot::channel();
    tokio::spawn(async move {
        tokio::select! {
            _ = &mut shutdown_rx => {},
            accepted = listener.accept() => if let Ok((stream, _)) = accepted {
                if let Ok(mut socket) = tokio_tungstenite::accept_async(stream).await {
                    while let Some(Ok(message)) = socket.next().await {
                        if socket.send(message).await.is_err() { break; }
                    }
                }
            }
        }
    });
    Ok((format!("ws://{address}"), shutdown_tx))
}

pub fn ffi_batch_callback(batch: &[Vec<u8>]) -> u64 {
    batch
        .iter()
        .flat_map(|event| event.iter())
        .fold(0_u64, |sum, byte| sum.wrapping_add(*byte as u64))
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
