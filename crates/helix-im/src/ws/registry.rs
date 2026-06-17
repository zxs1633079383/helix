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
        let handler_action = entry.handler.action();
        if entry.action != handler_action {
            return Err(ImError::InvalidWsHandlerRegistration {
                registered: entry.action.to_string(),
                handler: handler_action.to_string(),
            });
        }

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
    use crate::state::ImState;

    use super::*;

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
    static MISMATCHED_HANDLER: TestHandler = TestHandler {
        action: "post_update",
    };

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
    fn build_registry_rejects_registration_action_mismatch() {
        static ENTRIES: &[WsHandlerRegistration] = &[WsHandlerRegistration {
            action: "posted",
            handler: &MISMATCHED_HANDLER,
        }];

        let err = match build_registry(ENTRIES) {
            Ok(_) => panic!("registration action mismatch should fail"),
            Err(err) => err,
        };

        assert!(matches!(
            err,
            ImError::InvalidWsHandlerRegistration { registered, handler }
                if registered == "posted" && handler == "post_update"
        ));
    }

    #[test]
    fn distributed_registry_has_unique_actions() {
        let registry = build_registry(&WS_HANDLERS).unwrap();

        assert_eq!(registry.len(), WS_HANDLERS.len());
    }

    #[test]
    fn dispatch_ws_returns_unsupported_ws_action_for_unknown_action() {
        let mut state = ImState::new();
        let mut ctx = ImWsContext::new(&mut state);
        let frame = WsFrame::parse(br#"{"action":"missing"}"#).unwrap();

        let err = dispatch_ws(&mut ctx, &frame).unwrap_err();

        assert!(matches!(err, ImError::UnsupportedWsAction(action) if action == "missing"));
    }
}
