//! Shared host driver pieces for PC Tauri and Flutter FFI.

pub mod codec;
pub mod dispatch;
pub mod engine;
pub mod http_envelope;
pub mod network;
pub mod ops;
pub mod owned_effect;
pub mod pools;
pub mod spawner;
pub mod storage;
pub mod timer;

pub use codec::{rows_from_reply_bytes, rows_to_reply_bytes};
pub use engine::{run_engine_loop, EngineDeps, TransportTable};
pub use http_envelope::{base64_encode, run_http_envelope};
pub use network::{
    HostHeaderRegistry, HostNetworkConfig, PreparedHttpRequest, SharedHttpClient, SharedWsClient,
};
pub use ops::{classify_port_error, execute_storage_ops, table_set_key};
pub use owned_effect::{consume_effects, OwnedEffect};
pub use spawner::{BoundedSpawner, Job, Overflow};
pub use storage::HostStorage;
pub use timer::TimerRegistry;

/// per-table-set Persist worker 的有界 job 队列深度上限。
///
/// 超过此值时 dispatch 侧 `.await` 阻塞主循环，将背压传导到 tick_tx（cap=256）。
/// 这恢复了旧实现"磁盘慢则入站慢"的背压链，同时避免死锁（PortReply 走独立通道）。
pub const PERSIST_QUEUE_CAP: usize = 1024;

/// Http / HttpFire BoundedSpawner 的有界队列深度上限。
///
/// 必达 Http（Block）满时 `send().await` 把背压传导回 tick_tx；
/// HttpFire（DropOldest）满时 try_send 失败即丢最旧 + warn（自愈靠 cursor-gate 重发）。
pub const HTTP_QUEUE_CAP: usize = 1024;
