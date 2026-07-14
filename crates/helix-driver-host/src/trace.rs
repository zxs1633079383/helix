mod carrier;
mod hooks;

pub(crate) use carrier::parse_trace_id;
pub use carrier::{
    CommandTraceQueue, TraceCarrier, TraceCarrierError, TRACEPARENT_BYTES, TRACE_BAGGAGE_MAX_BYTES,
    TRACE_CARRIER_JSON_MAX_BYTES,
};
pub use hooks::{TraceHooks, TraceHooksImpl, TraceScope};

#[cfg(test)]
mod trace_tests;
