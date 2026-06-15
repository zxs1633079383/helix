//! Shared host driver pieces for PC Tauri and Flutter FFI.

pub mod codec;
pub mod ops;
pub mod storage;

pub use codec::{rows_from_reply_bytes, rows_to_reply_bytes};
pub use ops::{classify_port_error, execute_storage_ops, table_set_key};
pub use storage::HostStorage;
