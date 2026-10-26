//! 专用诊断旁路：业务线程仅复制有限标量，后台批量写 Loki，身份不进入索引。
mod cohort;
mod fields;
mod worker;

use crate::{AsyncMetricSink, ShutdownOutcome};
use fields::Fields;
use std::sync::{
    atomic::{AtomicU64, AtomicUsize, Ordering},
    mpsc, Arc,
};
use std::time::Duration;
use tokio::sync::{mpsc as queue, oneshot};
use tracing_subscriber::{layer::Context, Layer};

#[derive(Clone, Debug)]
pub struct DiagnosticConfig {
    pub enabled: bool,
    pub loki_url: String,
    pub environment: String,
    pub platform: String,
    pub producer_service: String,
    pub source_revision: String,
    pub queue_capacity: usize,
    pub batch_max: usize,
    pub export_interval: Duration,
    pub timeout: Duration,
    pub retry_max: u32,
}
impl Default for DiagnosticConfig {
    /// 默认关闭；端点只能由宿主显式配置。
    fn default() -> Self {
        Self {
            enabled: false,
            loki_url: String::new(),
            environment: "local".into(),
            platform: "host".into(),
            producer_service: "helix-driver-host".into(),
            source_revision: "unknown".into(),
            queue_capacity: 1024,
            batch_max: 64,
            export_interval: Duration::from_secs(1),
            timeout: Duration::from_secs(2),
            retry_max: 2,
        }
    }
}
impl DiagnosticConfig {
    /// 读取宿主环境，禁止从业务帧或renderer选择导出端点。
    pub fn from_env() -> Self {
        Self {
            enabled: std::env::var("HELIX_DIAGNOSTICS_ENABLED")
                .is_ok_and(|v| v == "true" || v == "1"),
            loki_url: std::env::var("HELIX_DIAGNOSTICS_LOKI_URL").unwrap_or_default(),
            environment: std::env::var("HELIX_DIAGNOSTICS_ENVIRONMENT")
                .unwrap_or_else(|_| "local".into()),
            platform: std::env::var("HELIX_DIAGNOSTICS_PLATFORM").unwrap_or_else(|_| "host".into()),
            producer_service: std::env::var("HELIX_DIAGNOSTICS_SERVICE")
                .unwrap_or_else(|_| "helix-driver-host".into()),
            source_revision: std::env::var("HELIX_DIAGNOSTICS_REVISION")
                .unwrap_or_else(|_| "unknown".into()),
            ..Self::default()
        }
    }
}

#[derive(Default)]
struct Stats {
    accepted: AtomicU64,
    exported: AtomicU64,
    dropped: AtomicU64,
    invalid: AtomicU64,
    errors: AtomicU64,
    depth: AtomicUsize,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct DiagnosticSnapshot {
    pub accepted: u64,
    pub exported: u64,
    pub dropped: u64,
    pub invalid: u64,
    pub errors: u64,
    pub queue_depth: usize,
}
impl Stats {
    /// 单次读取自观测计数，不扫描业务状态。
    fn snapshot(&self) -> DiagnosticSnapshot {
        DiagnosticSnapshot {
            accepted: self.accepted.load(Ordering::Relaxed),
            exported: self.exported.load(Ordering::Relaxed),
            dropped: self.dropped.load(Ordering::Relaxed),
            invalid: self.invalid.load(Ordering::Relaxed),
            errors: self.errors.load(Ordering::Relaxed),
            queue_depth: self.depth.load(Ordering::Relaxed),
        }
    }
}

#[derive(Clone)]
pub struct DiagnosticLayer {
    tx: Option<queue::Sender<Fields>>,
    stats: Arc<Stats>,
}
impl<S: tracing::Subscriber> Layer<S> for DiagnosticLayer {
    /// 仅收专用target；固定栈字段+try_send，不执行JSON/网络/磁盘。
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        if event.metadata().target() != "helix::diagnostics" {
            return;
        }
        let Some(tx) = &self.tx else {
            return;
        };
        let mut fields = Fields::default();
        event.record(&mut fields);
        if fields.invalid || !fields.has_event() {
            self.stats.invalid.fetch_add(1, Ordering::Relaxed);
            return;
        }
        match tx.try_reserve() {
            Ok(permit) => {
                self.stats.depth.fetch_add(1, Ordering::Relaxed);
                self.stats.accepted.fetch_add(1, Ordering::Relaxed);
                permit.send(fields);
            }
            Err(_) => {
                self.stats.dropped.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

pub struct DiagnosticRuntime {
    layer: DiagnosticLayer,
    stop: Option<oneshot::Sender<()>>,
    done: Option<mpsc::Receiver<()>>,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl DiagnosticRuntime {
    /// 在独立线程拥有异步HTTP运行时；初始化失败由宿主降级，绝不改变业务结果。
    pub fn start(
        mut config: DiagnosticConfig,
        metrics: Arc<dyn AsyncMetricSink>,
    ) -> std::io::Result<Self> {
        let stats = Arc::new(Stats::default());
        if !config.enabled {
            return Ok(Self {
                layer: DiagnosticLayer { tx: None, stats },
                stop: None,
                done: None,
                worker: None,
            });
        }
        let endpoint = reqwest::Url::parse(&config.loki_url).map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "invalid diagnostic Loki URL",
            )
        })?;
        if !matches!(endpoint.scheme(), "http" | "https")
            || !endpoint.username().is_empty()
            || endpoint.password().is_some()
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "invalid diagnostic endpoint scheme or embedded credentials",
            ));
        }
        config.queue_capacity = config.queue_capacity.clamp(1, 4096);
        config.batch_max = config.batch_max.clamp(1, 256);
        config.retry_max = config.retry_max.min(3);
        config.timeout = config
            .timeout
            .clamp(Duration::from_millis(100), Duration::from_secs(10));
        config.export_interval = config
            .export_interval
            .clamp(Duration::from_millis(10), Duration::from_secs(10));
        let (tx, rx) = queue::channel(config.queue_capacity);
        let (stop, stopped) = oneshot::channel();
        let (done_tx, done) = mpsc::channel();
        let worker_stats = Arc::clone(&stats);
        let worker = std::thread::Builder::new()
            .name("helix-diagnostics".into())
            .spawn(move || {
                if let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    runtime.block_on(worker::run(config, rx, stopped, worker_stats, metrics));
                }
                let _ = done_tx.send(());
            })?;
        Ok(Self {
            layer: DiagnosticLayer {
                tx: Some(tx),
                stats,
            },
            stop: Some(stop),
            done: Some(done),
            worker: Some(worker),
        })
    }
    /// Subscriber持有独立sender；shutdown会关闭receiver并拒绝迟到记录。
    pub fn layer(&self) -> DiagnosticLayer {
        self.layer.clone()
    }
    /// 返回采集尝试的真实计数，不代表业务消息送达。
    pub fn snapshot(&self) -> DiagnosticSnapshot {
        self.layer.stats.snapshot()
    }
    /// 有限排空，超时明确返回TimedOut，不能冒充worker已退出。
    pub fn shutdown(&mut self, timeout: Duration) -> ShutdownOutcome {
        if self.layer.tx.is_none() {
            return ShutdownOutcome::Disabled;
        }
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if self
            .done
            .as_ref()
            .is_some_and(|done| done.recv_timeout(timeout).is_ok())
        {
            if let Some(worker) = self.worker.take() {
                let _ = worker.join();
            }
            self.done = None;
            ShutdownOutcome::Drained
        } else {
            ShutdownOutcome::TimedOut
        }
    }
}
impl Drop for DiagnosticRuntime {
    /// Drop只请求关闭，不在业务退出路径无限等待网络。
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
    }
}

static ACTIVE: std::sync::OnceLock<arc_swap::ArcSwapOption<DiagnosticLayer>> =
    std::sync::OnceLock::new();
/// 宿主一次安装的无锁路由；各认证runtime可切换自己的sink，身份仍来自事件正文。
pub struct GlobalDiagnosticLayer;
impl<S: tracing::Subscriber> Layer<S> for GlobalDiagnosticLayer {
    /// 原子读取当前runtime的有界sink，不在消息热路径取得互斥锁。
    fn on_event(&self, event: &tracing::Event<'_>, ctx: Context<'_, S>) {
        if event.metadata().target() != "helix::diagnostics" {
            return;
        }
        if let Some(active) = ACTIVE.get().and_then(|slot| slot.load_full()) {
            active.on_event(event, ctx);
        }
    }
}
impl DiagnosticRuntime {
    /// 认证runtime启动后绑定已创建的metrics和日志旁路；旧runtime停止不会移除新绑定。
    pub fn activate_global(&self) {
        ACTIVE
            .get_or_init(arc_swap::ArcSwapOption::empty)
            .store(Some(Arc::new(self.layer())));
    }
}
/// 仅在宿主冷启动生成本次诊断会话ID，不属于业务状态机或业务event key。
pub fn new_session_id() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!(
        "{}-{now}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}
/// 同一进程设备会话固定，登录/租户runtime另分login_attempt_id。
pub fn device_session_id() -> &'static str {
    static ID: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    ID.get_or_init(new_session_id).as_str()
}
