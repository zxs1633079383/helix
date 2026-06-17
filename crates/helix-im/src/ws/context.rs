use helix_core::ports::EventSink;

use crate::state::ImState;

pub(crate) struct ImWsContext<'a> {
    pub(crate) state: &'a mut ImState,
    pub(crate) event_sink: &'a dyn EventSink,
}

impl<'a> ImWsContext<'a> {
    pub(crate) fn new(state: &'a mut ImState, event_sink: &'a dyn EventSink) -> Self {
        Self { state, event_sink }
    }
}
