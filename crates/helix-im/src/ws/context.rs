use crate::state::ImState;
use helix_core::Correlation;

pub(crate) struct ImWsContext<'a> {
    pub(crate) state: &'a mut ImState,
    pub(crate) now_ms: u64,
    pub(crate) api_base_url: &'a str,
    alloc_corr: &'a mut dyn FnMut() -> Correlation,
}

impl<'a> ImWsContext<'a> {
    pub(crate) fn new(
        state: &'a mut ImState,
        now_ms: u64,
        api_base_url: &'a str,
        alloc_corr: &'a mut dyn FnMut() -> Correlation,
    ) -> Self {
        Self {
            state,
            now_ms,
            api_base_url,
            alloc_corr,
        }
    }

    pub(crate) fn alloc_corr(&mut self) -> Correlation {
        (self.alloc_corr)()
    }
}
