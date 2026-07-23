//! # helix-driver-native
//!
//! tokio shell + shared host storage/network implementations for helix-core ports.
//!
//! ## tokio::spawn 在此
//!
//! helix-core 不含任何运行时依赖。`tokio::spawn` 在此 crate 的 `engine_loop.rs`，
//! 不在 helix-core。
//!
//! ## 错误翻译（ACL-2）
//!
//! 具体 I/O crate 错误在 driver/host 边界翻译为 helix_core::PortError，
//! 不让具体类型穿透 port 边界。

pub mod bus_guard;
pub mod clock;
pub mod engine_loop;
pub mod event_sink;
pub mod http;
pub mod storage;
pub mod transport;

// 公共 API 再导出（方便 helix-tauri / 测试直接 use helix_driver_native::*）
pub use bus_guard::{guard_bus_envelope, is_emittable_channel, GuardedEnvelope};
pub use clock::NativeClock;
pub use engine_loop::{
    rows_from_reply_bytes, rows_to_reply_bytes, run_engine_loop, run_engine_loop_stamped,
    run_engine_loop_with_transports, run_engine_loop_with_transports_and_trace,
    run_engine_loop_with_transports_and_trace_stamped, EngineConfig, TransportTable,
};
pub use event_sink::{
    bus_event_name, to_bus_envelope, EventReceiver, NativeEventSink, RecvOutcome, BUS_CHANNEL,
};
pub use http::NativeHttp;
pub use storage::NativeStorage;
pub use transport::NativeTransport;
// TimerRegistry 已下沉 host（pump 核统一）；native re-export 保持公开路径兼容。
pub use helix_driver_host::TimerRegistry;
// 共享 host I/O 配置（ADR-010）：Go base 由 new() 注入，Java/default base 必须再经
// with_default_api_base_url() 显式注入；禁止从 Go URL 推测。re-export 免直依 driver-host。
pub use helix_driver_host::HostNetworkConfig;
// P1.5 出站横切层：装配 NativeHttp::with_crosscut 时注入真鉴权 token 源（re-export 免直依 driver-host）。
pub use helix_driver_host::AuthTokenRegistry;
// 共享 HTTP client 句柄：host 装配 NativeHttp::with_crosscut 需先 SharedHttpClient::new(config)，
// 再连同 event_sink + AuthTokenRegistry 包进横切层（re-export 免直依 driver-host）。
pub use helix_driver_host::SharedHttpClient;
// 共享本地文件上传器：host-cli / shell 通过 native 装配 `EngineConfig.uploader`，免直依 driver-host。
pub use helix_driver_host::SharedFileUploader;
pub use helix_driver_host::{tick_ingress_channel, TickIngressReceiver, TickIngressSender};
