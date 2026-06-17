use crate::state::ImState;

pub(crate) struct ImWsContext<'a> {
    pub(crate) state: &'a mut ImState,
    pub(crate) now_ms: u64,
}

impl<'a> ImWsContext<'a> {
    pub(crate) fn new(state: &'a mut ImState, now_ms: u64) -> Self {
        Self { state, now_ms }
    }
}
