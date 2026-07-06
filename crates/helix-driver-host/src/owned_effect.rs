//! OwnedEffect — `&[Effect]` 借用脱离镜像。
//!
//! `ExecutionShell::step()` 返回 `&[Effect]`，借用 `shell.scratch`（跨 step 复用 Vec）。
//! 下次 `step()` 前必须消费完。策略：立即 [`consume_effects`] 把 `&[Effect]` 转成
//! `Vec<OwnedEffect>`（owned struct，按需 Clone 字段），脱离借用后再 async 兑现。
//!
//! `Effect` 自身不实现 Clone（持有 non-Clone Bytes 和 Vec），但 `bytes::Bytes`
//! 是 O(1) clone（引用计数），其他字段都可 clone。

use bytes::Bytes;

use helix_core::effect::{
    Correlation, DomainEventBytes, Effect, FileUploadRequest, HttpRequest, StorageOp, TimerId,
    TransportId,
};

/// `Effect` 的 owned 镜像：脱离 `shell.scratch` 借用后承载到 async 兑现阶段。
pub enum OwnedEffect {
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

/// 立即从 `&[Effect]` 转换为 `Vec<OwnedEffect>`（脱离借用）。
///
/// `bytes::Bytes` clone 是 O(1) 引用计数；`Vec` clone 按需复制。
/// 此函数同步执行，在 shell 返回 `&[Effect]` 后立即调用。
pub fn consume_effects(effects: &[Effect]) -> Vec<OwnedEffect> {
    effects
        .iter()
        .map(|e| match e {
            Effect::Send { transport, frame } => OwnedEffect::Send {
                transport: *transport,
                frame: frame.clone(),
            },
            Effect::Persist { corr, ops } => OwnedEffect::Persist {
                corr: *corr,
                ops: ops.clone(),
            },
            Effect::PersistFire { ops } => OwnedEffect::PersistFire { ops: ops.clone() },
            Effect::Http { corr, req } => OwnedEffect::Http {
                corr: *corr,
                req: req.clone(),
            },
            Effect::UploadFile { corr, req } => OwnedEffect::UploadFile {
                corr: *corr,
                req: req.clone(),
            },
            Effect::HttpFire { req } => OwnedEffect::HttpFire { req: req.clone() },
            Effect::Request {
                corr,
                kind,
                payload,
            } => OwnedEffect::Request {
                corr: *corr,
                kind,
                payload: payload.clone(),
            },
            Effect::Emit { event } => OwnedEffect::Emit {
                event: event.clone(),
            },
            Effect::ScheduleTimer { id, after_ms } => OwnedEffect::ScheduleTimer {
                id: *id,
                after_ms: *after_ms,
            },
            Effect::CancelTimer { id } => OwnedEffect::CancelTimer { id: *id },
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_consume_effects_http_and_httpfire_roundtrip() {
        let req = || HttpRequest {
            method: "POST".to_string(),
            url: "http://test/x".to_string(),
            headers: vec![],
            body: None,
        };
        let effects = vec![
            Effect::Http {
                corr: Correlation::from_raw(7),
                req: req(),
            },
            Effect::HttpFire { req: req() },
        ];
        let owned = consume_effects(&effects);
        assert_eq!(owned.len(), 2);
        match &owned[0] {
            OwnedEffect::Http { corr, req } => {
                assert_eq!(corr.raw(), 7);
                assert_eq!(req.url, "http://test/x");
            }
            _ => panic!("期望 OwnedEffect::Http"),
        }
        match &owned[1] {
            OwnedEffect::HttpFire { req } => assert_eq!(req.method, "POST"),
            _ => panic!("期望 OwnedEffect::HttpFire"),
        }
    }
}
