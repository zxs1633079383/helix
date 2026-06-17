use std::collections::HashMap;
use std::sync::OnceLock;

use linkme::distributed_slice;

use crate::error::ImError;

use super::context::ImWsContext;
use super::frame::WsFrame;

pub(crate) type ImEffect = helix_core::Effect;

pub(crate) trait WsMessageHandler: Sync {
    fn action(&self) -> &'static str;

    fn handle(&self, ctx: &mut ImWsContext<'_>, frame: &WsFrame) -> Result<Vec<ImEffect>, ImError>;
}

pub(crate) struct WsHandlerRegistration {
    pub(crate) action: &'static str,
    pub(crate) handler: &'static dyn WsMessageHandler,
}

#[distributed_slice]
pub(crate) static WS_HANDLERS: [WsHandlerRegistration];

static WS_HANDLER_MAP: OnceLock<
    Result<HashMap<&'static str, &'static dyn WsMessageHandler>, ImError>,
> = OnceLock::new();

pub(crate) fn registry(
) -> Result<&'static HashMap<&'static str, &'static dyn WsMessageHandler>, ImError> {
    WS_HANDLER_MAP
        .get_or_init(|| build_registry(&WS_HANDLERS))
        .as_ref()
        .map_err(Clone::clone)
}

fn build_registry(
    entries: &'static [WsHandlerRegistration],
) -> Result<HashMap<&'static str, &'static dyn WsMessageHandler>, ImError> {
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
    frame: &WsFrame,
) -> Result<Vec<ImEffect>, ImError> {
    let handler = registry()?
        .get(frame.action())
        .ok_or_else(|| ImError::UnsupportedWsAction(frame.action().to_string()))?;

    handler.handle(ctx, frame)
}

#[cfg(test)]
mod tests {
    use helix_core::ports::EventSink;
    use helix_core::DomainEventBytes;

    use crate::state::ImState;

    use super::*;

    struct NoopEventSink;

    impl EventSink for NoopEventSink {
        fn emit(&self, _event: DomainEventBytes) {}
    }

    struct TestHandler {
        action: &'static str,
    }

    impl WsMessageHandler for TestHandler {
        fn action(&self) -> &'static str {
            self.action
        }

        fn handle(
            &self,
            _ctx: &mut ImWsContext<'_>,
            _frame: &WsFrame,
        ) -> Result<Vec<ImEffect>, ImError> {
            Ok(Vec::new())
        }
    }

    static POSTED_HANDLER: TestHandler = TestHandler { action: "posted" };
    static DUPLICATE_POSTED_HANDLER: TestHandler = TestHandler { action: "posted" };

    #[test]
    fn build_registry_rejects_duplicate_actions() {
        static ENTRIES: &[WsHandlerRegistration] = &[
            WsHandlerRegistration {
                action: "posted",
                handler: &POSTED_HANDLER,
            },
            WsHandlerRegistration {
                action: "posted",
                handler: &DUPLICATE_POSTED_HANDLER,
            },
        ];

        let err = match build_registry(ENTRIES) {
            Ok(_) => panic!("duplicate actions should fail"),
            Err(err) => err,
        };

        assert!(matches!(err, ImError::DuplicateWsAction(action) if action == "posted"));
    }

    #[test]
    fn distributed_registry_has_unique_actions() {
        let registry = build_registry(&WS_HANDLERS).unwrap();

        assert_eq!(registry.len(), WS_HANDLERS.len());
    }

    #[test]
    fn dispatch_ws_returns_unsupported_ws_action_for_unknown_action() {
        let mut state = ImState::new();
        let event_sink = NoopEventSink;
        let mut ctx = ImWsContext::new(&mut state, &event_sink);
        let frame = WsFrame::parse(br#"{"action":"missing"}"#).unwrap();

        let err = dispatch_ws(&mut ctx, &frame).unwrap_err();

        assert!(matches!(err, ImError::UnsupportedWsAction(action) if action == "missing"));
    }
}
