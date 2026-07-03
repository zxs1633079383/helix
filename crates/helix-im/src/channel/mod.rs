//! Channel aggregate root and channel write model.

pub mod gate;
mod lifecycle;
mod lifecycle_scan;
pub mod older_context;
pub mod write;

pub use gate::*;
