use crate::module::ImModule;

pub(crate) struct ImWsContext<'a> {
    pub(crate) module: &'a mut ImModule,
    pub(crate) now_ms: u64,
}

impl<'a> ImWsContext<'a> {
    pub(crate) fn new(module: &'a mut ImModule, now_ms: u64) -> Self {
        Self { module, now_ms }
    }
}
