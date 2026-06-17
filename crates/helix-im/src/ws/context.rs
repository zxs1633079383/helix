use crate::state::ImState;

pub(crate) struct ImWsContext<'a> {
    pub(crate) state: &'a mut ImState,
}

impl<'a> ImWsContext<'a> {
    pub(crate) fn new(state: &'a mut ImState) -> Self {
        Self { state }
    }
}
