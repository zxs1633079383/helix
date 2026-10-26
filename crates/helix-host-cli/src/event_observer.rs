use helix_core::effect::DomainEventBytes;
use tokio::sync::broadcast;
use tokio::sync::broadcast::error::RecvError;

pub fn spawn(mut receiver: broadcast::Receiver<DomainEventBytes>) {
    tokio::spawn(async move {
        loop {
            match receiver.recv().await {
                Ok(event) => log_event(&event),
                Err(RecvError::Lagged(dropped)) => {
                    tracing::warn!(dropped, "EventSink 落后丢事件（应触发重拉）")
                }
                Err(RecvError::Closed) => break,
            }
        }
    });
}

fn log_event(event: &DomainEventBytes) {
    let name = serde_json::from_slice::<serde_json::Value>(&event.0)
        .ok()
        .and_then(|value| {
            value
                .get("event")
                .and_then(|item| item.as_str())
                .map(str::to_owned)
        });
    match name {
        Some(name) => tracing::info!(event = %name, bytes = event.0.len(), "DomainEvent emit"),
        None => tracing::info!(bytes = event.0.len(), "DomainEvent emit (unparsed)"),
    }
}
