//! Browser-friendly owned shell boundary.
//!
//! `ExecutionShell` reuses a scratch buffer and returns `&[Effect]`. The Web
//! runtime must release that borrow before spawning Fetch or accepting another
//! browser callback, so every step is copied immediately into this private
//! owned mirror.

use bytes::Bytes;
use helix_core::effect::{
    Correlation, DomainEventBytes, FileUploadRequest, HttpRequest, StorageOp, TimerId, TransportId,
};
use helix_core::{CoreError, Effect, ExecutionShell, Tick};

#[derive(Debug)]
pub enum WebOwnedEffect {
    Send {
        transport: TransportId,
        frame: Bytes,
    },
    Persist {
        corr: Correlation,
        ops: Vec<StorageOp>,
    },
    PersistFire {
        ops: Vec<StorageOp>,
    },
    Http {
        corr: Correlation,
        req: HttpRequest,
    },
    UploadFile {
        corr: Correlation,
        req: FileUploadRequest,
    },
    HttpFire {
        req: HttpRequest,
    },
    Request {
        corr: Correlation,
        kind: &'static str,
        payload: Bytes,
    },
    Emit {
        event: DomainEventBytes,
    },
    ScheduleTimer {
        id: TimerId,
        after_ms: u64,
    },
    CancelTimer {
        id: TimerId,
    },
}

pub struct WebExecutor {
    shell: ExecutionShell,
    started: bool,
    stopped: bool,
}

impl WebExecutor {
    pub fn new(shell: ExecutionShell) -> Self {
        Self {
            shell,
            started: false,
            stopped: false,
        }
    }

    pub fn start(&mut self) -> Result<Vec<WebOwnedEffect>, CoreError> {
        if self.started || self.stopped {
            return Ok(Vec::new());
        }
        self.started = true;
        self.shell.start().map(own_effects)
    }

    pub fn step(&mut self, tick: Tick, now_ms: u64) -> Result<Vec<WebOwnedEffect>, CoreError> {
        if self.stopped {
            return Ok(Vec::new());
        }
        self.shell.step(tick, now_ms).map(own_effects)
    }

    pub fn stop(&mut self) -> Result<Vec<WebOwnedEffect>, CoreError> {
        if self.stopped {
            return Ok(Vec::new());
        }
        self.stopped = true;
        self.shell.stop().map(own_effects)
    }
}

fn own_effects(effects: &[Effect]) -> Vec<WebOwnedEffect> {
    effects.iter().map(WebOwnedEffect::from).collect()
}

impl From<&Effect> for WebOwnedEffect {
    fn from(effect: &Effect) -> Self {
        match effect {
            Effect::Send { transport, frame } => Self::Send {
                transport: *transport,
                frame: frame.clone(),
            },
            Effect::Persist { corr, ops } => Self::Persist {
                corr: *corr,
                ops: ops.clone(),
            },
            Effect::PersistFire { ops } => Self::PersistFire { ops: ops.clone() },
            Effect::Http { corr, req } => Self::Http {
                corr: *corr,
                req: req.clone(),
            },
            Effect::UploadFile { corr, req } => Self::UploadFile {
                corr: *corr,
                req: req.clone(),
            },
            Effect::HttpFire { req } => Self::HttpFire { req: req.clone() },
            Effect::Request {
                corr,
                kind,
                payload,
            } => Self::Request {
                corr: *corr,
                kind,
                payload: payload.clone(),
            },
            Effect::Emit { event } => Self::Emit {
                event: event.clone(),
            },
            Effect::ScheduleTimer { id, after_ms } => Self::ScheduleTimer {
                id: *id,
                after_ms: *after_ms,
            },
            Effect::CancelTimer { id } => Self::CancelTimer { id: *id },
        }
    }
}

#[cfg(test)]
mod tests {
    use helix_core::effect::{
        BatchDeleteSpec, DomainEventBytes, FileUploadRequest, FileUploadUrls, HttpRequest,
        SqlValue, StorageOp,
    };

    use super::*;

    #[test]
    fn owned_boundary_mirrors_every_effect_variant_without_reordering() {
        let http = HttpRequest {
            method: "POST".into(),
            url: "https://example.invalid".into(),
            headers: vec![("x-test".into(), "1".into())],
            body: Some(Bytes::from_static(b"body")),
        };
        let storage = StorageOp::BatchDelete(BatchDeleteSpec {
            table: "table",
            scope_col: "scope",
            scope_val: SqlValue::Text("test".into()),
            key_col: "id",
            key_vals: vec![SqlValue::Text("1".into())],
        });
        let effects = vec![
            Effect::Send {
                transport: TransportId::from_raw(3),
                frame: Bytes::from_static(b"frame"),
            },
            Effect::Persist {
                corr: Correlation::from_raw(1),
                ops: vec![storage.clone()],
            },
            Effect::PersistFire { ops: vec![storage] },
            Effect::Http {
                corr: Correlation::from_raw(2),
                req: http.clone(),
            },
            Effect::UploadFile {
                corr: Correlation::from_raw(3),
                req: FileUploadRequest {
                    local_path: "/tmp/file".into(),
                    object_key: "object".into(),
                    method: "PUT".into(),
                    urls: FileUploadUrls::new(
                        "https://example.invalid/upload".into(),
                        "https://example.invalid/file".into(),
                    )
                    .expect("valid urls"),
                    headers: Vec::new(),
                    content_type: None,
                    size: Some(4),
                },
            },
            Effect::HttpFire { req: http },
            Effect::Request {
                corr: Correlation::from_raw(4),
                kind: "host_config",
                payload: Bytes::from_static(b"request"),
            },
            Effect::Emit {
                event: DomainEventBytes(Bytes::from_static(b"event")),
            },
            Effect::ScheduleTimer {
                id: TimerId::from_raw(5),
                after_ms: 6,
            },
            Effect::CancelTimer {
                id: TimerId::from_raw(7),
            },
        ];

        let owned = own_effects(&effects);
        assert_eq!(owned.len(), effects.len());
        assert_eq!(format!("{effects:?}"), format!("{owned:?}"));
    }
}
