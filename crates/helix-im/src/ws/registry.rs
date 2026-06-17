use std::collections::HashMap;
use std::sync::OnceLock;

use helix_core::EffectSink;
use linkme::distributed_slice;

use crate::error::ImError;

use super::context::ImWsContext;
use super::frame::WsFrame;

pub(crate) type WsHandlerFn =
    fn(&mut ImWsContext<'_>, &WsFrame, &mut EffectSink) -> Result<(), ImError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WsActionKind {
    Business,
    Ignored,
}

pub(crate) struct WsHandlerEntry {
    pub(crate) action: &'static str,
    pub(crate) kind: WsActionKind,
    pub(crate) handler: WsHandlerFn,
}

#[distributed_slice]
pub(crate) static WS_HANDLERS: [WsHandlerEntry];

static WS_HANDLER_MAP: OnceLock<Result<HashMap<&'static str, WsHandlerFn>, ImError>> =
    OnceLock::new();

pub(crate) fn registry() -> Result<&'static HashMap<&'static str, WsHandlerFn>, ImError> {
    WS_HANDLER_MAP
        .get_or_init(|| build_registry(&WS_HANDLERS))
        .as_ref()
        .map_err(Clone::clone)
}

fn build_registry(
    entries: &'static [WsHandlerEntry],
) -> Result<HashMap<&'static str, WsHandlerFn>, ImError> {
    let mut registry = HashMap::with_capacity(entries.len());

    for entry in entries {
        if registry.insert(entry.action, entry.handler).is_some() {
            return Err(ImError::DuplicateWsAction(entry.action.to_string()));
        }
    }

    Ok(registry)
}

pub(crate) fn dispatch_ws(
    ctx: &mut ImWsContext<'_>,
    raw: &[u8],
    out: &mut EffectSink,
) -> Result<(), ImError> {
    let frame = WsFrame::parse(raw)?;
    let handler = registry()?
        .get(frame.action())
        .ok_or_else(|| ImError::UnsupportedWsAction(frame.action().to_string()))?;

    handler(ctx, &frame, out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn noop_handler(
        _ctx: &mut ImWsContext<'_>,
        _frame: &WsFrame,
        _out: &mut EffectSink,
    ) -> Result<(), ImError> {
        Ok(())
    }

    #[test]
    fn build_registry_rejects_duplicate_actions() {
        static ENTRIES: &[WsHandlerEntry] = &[
            WsHandlerEntry {
                action: "posted",
                kind: WsActionKind::Business,
                handler: noop_handler,
            },
            WsHandlerEntry {
                action: "posted",
                kind: WsActionKind::Ignored,
                handler: noop_handler,
            },
        ];

        let err = build_registry(ENTRIES).unwrap_err();

        assert!(matches!(err, ImError::DuplicateWsAction(action) if action == "posted"));
    }

    #[test]
    fn distributed_registry_has_unique_actions() {
        let registry = build_registry(&WS_HANDLERS).unwrap();

        assert_eq!(registry.len(), WS_HANDLERS.len());
    }
}
