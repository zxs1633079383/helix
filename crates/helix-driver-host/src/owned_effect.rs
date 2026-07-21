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
    PersistAtomic {
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

/// owned 转换阶段顺带产出的固定大小摘要。
///
/// 摘要只含标量，不为指标另建集合；engine 后续记录批次指标时无需再次扫描 effect。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EffectSummary {
    pub count: usize,
    pub bytes: usize,
    pub event_count: usize,
}

pub struct OwnedEffectBatch {
    pub effects: Vec<OwnedEffect>,
    pub summary: EffectSummary,
}

/// 立即从 `&[Effect]` 转换为 `Vec<OwnedEffect>`（脱离借用）。
///
/// `bytes::Bytes` clone 是 O(1) 引用计数；`Vec` clone 按需复制。
/// 此函数同步执行，在 shell 返回 `&[Effect]` 后立即调用。
pub fn consume_effects(effects: &[Effect]) -> Vec<OwnedEffect> {
    consume_effect_batch(effects).effects
}

/// 单趟完成 borrowed effect → owned effect 转换与指标摘要。
pub fn consume_effect_batch(effects: &[Effect]) -> OwnedEffectBatch {
    let mut owned = Vec::with_capacity(effects.len());
    let mut summary = EffectSummary::default();
    for effect in effects {
        summary.count += 1;
        summary.bytes += effect_bytes(effect);
        summary.event_count += usize::from(matches!(effect, Effect::Emit { .. }));
        owned.push(match effect {
            Effect::Send { transport, frame } => OwnedEffect::Send {
                transport: *transport,
                frame: frame.clone(),
            },
            Effect::Persist { corr, ops } => OwnedEffect::Persist {
                corr: *corr,
                ops: ops.clone(),
            },
            Effect::PersistAtomic { corr, ops } => OwnedEffect::PersistAtomic {
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
        });
    }
    OwnedEffectBatch {
        effects: owned,
        summary,
    }
}

fn effect_bytes(effect: &Effect) -> usize {
    match effect {
        Effect::Send { frame, .. } => frame.len(),
        Effect::Persist { ops, .. }
        | Effect::PersistAtomic { ops, .. }
        | Effect::PersistFire { ops } => ops.len() * std::mem::size_of::<StorageOp>(),
        Effect::Http { req, .. } | Effect::HttpFire { req } => {
            req.body.as_ref().map_or(0, Bytes::len)
        }
        Effect::UploadFile { req, .. } => req.size.unwrap_or(0) as usize,
        Effect::Request { payload, .. } => payload.len(),
        Effect::Emit { event } => event.0.len(),
        Effect::ScheduleTimer { .. } | Effect::CancelTimer { .. } => 0,
    }
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

    #[test]
    fn consume_effect_batch_summarizes_without_second_scan() {
        let effects = vec![
            Effect::Send {
                transport: TransportId::from_raw(1),
                frame: Bytes::from_static(b"abc"),
            },
            Effect::Emit {
                event: DomainEventBytes(Bytes::from_static(b"event")),
            },
        ];
        let batch = consume_effect_batch(&effects);
        assert_eq!(batch.effects.len(), 2);
        assert_eq!(batch.summary.count, 2);
        assert_eq!(batch.summary.bytes, 8);
        assert_eq!(batch.summary.event_count, 1);
    }
}
