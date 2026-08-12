//! BoundedSpawner — host 内通用有界并发原语（刀2-driver）。
//!
//! Persist / Http(必达) / HttpFire(可丢) 统一成此原语：有界 mpsc 队列(cap=K) + N 个
//! 常驻 worker task + 溢出策略 + 独立 unbounded reply_tx 回灌。三类只是参数差异，
//! 背压纪律集中在一处（Dioxus「统一流」取舍：一个背压/批次/调度策略管全部）。
//!
//! ## 死锁安全（与现有 Persist 模型同构，已压测验证）
//!
//! 抽水在独立 worker task（不是泵自己 drain），PortReply 走独立 unbounded reply_tx，
//! 故 Block 模式泵阻塞在「入队」是安全的：依赖链是单向 DAG（泵等 worker，worker 不等泵）。
//! 🔴 两条红线：① reply_tx 必须 unbounded（不变量1）② 抽水绝不放泵循环（不变量2）。

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;

use tokio::sync::{mpsc, Mutex};
use tokio::task::JoinHandle;

use helix_core::effect::Correlation;
use helix_core::tick::PortOutcome;
use helix_core::Tick;

use crate::metrics::{
    AsyncMetricSink, LabelKey, MetricEvent, MetricId, MetricLabels, NoopMetricSink,
};

/// Engine 专用回灌元素：Tick 与完成入队时刻同处一条 unbounded 消息。
pub(crate) struct StampedFeedback {
    pub(crate) tick: Tick,
    pub(crate) enqueued_at: Instant,
}

/// Engine 专用回灌发送端；保持 unbounded 死锁不变量，只增加同元素时间戳。
#[derive(Clone)]
pub(crate) struct FeedbackSender {
    tx: mpsc::UnboundedSender<StampedFeedback>,
}

/// 创建带时间戳的独立回灌通道，不改变其 unbounded 必达语义。
pub(crate) fn feedback_channel() -> (FeedbackSender, mpsc::UnboundedReceiver<StampedFeedback>) {
    let (tx, rx) = mpsc::unbounded_channel();
    (FeedbackSender { tx }, rx)
}

#[derive(Clone)]
pub(crate) enum FeedbackSink {
    Raw(mpsc::UnboundedSender<Tick>),
    Stamped(FeedbackSender),
}

/// 溢出策略：有界队列满时怎么办（每实例固定一种）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Overflow {
    /// 队满 `send().await` 阻塞泵 → 背压传导回 tick_tx（慢而不丢）。Persist / Http(必达) 用。
    Block,
    /// 队满 try_send 失败即丢**当前（最新）**条目（warn，泵不阻塞）。HttpFire 用（丢失靠 cursor-gate 重发自愈）。
    ///
    /// 语义：`try_send` 满时丢的是**本次**（新来的）请求，不是队列里最旧的那条。
    /// 在 fire-and-forget 自愈场景下等价（两端都靠 cursor-gate 重发补），取最简实现（try_send-Full-drop）。
    DropNewest,
}

/// 一条工作单元：可选回报 corr（None = fire-and-forget，不发 PortReply）+ job 载荷。
pub struct Job<P> {
    pub corr: Option<Correlation>,
    pub payload: P,
}

/// 为 worker 内部保留真实入队时刻，不把 driver 时间概念泄漏到公开 Job 载荷。
struct QueuedJob<P> {
    job: Job<P>,
    enqueued_at: Option<Instant>,
}

/// 工作池的低基数观测上下文；所有写入都委托有界 AsyncMetricSink。
#[derive(Clone)]
struct SpawnerMetrics {
    sink: Arc<dyn AsyncMetricSink>,
    pool: &'static str,
    overflow: &'static str,
    depth: Arc<AtomicUsize>,
    inflight: Arc<AtomicUsize>,
    enabled: bool,
}

/// 通用有界并发原语：有界 mpsc(cap=K) + N 个常驻 worker + 溢出策略。
pub struct BoundedSpawner<P: Send + 'static> {
    /// 有界 job 发送端（cap = queue_cap）。
    tx: mpsc::Sender<QueuedJob<P>>,
    /// N 个 worker task 句柄，shutdown 时 join 确保积压（必达类）全部落地。
    joins: Vec<JoinHandle<()>>,
    /// 溢出策略。
    overflow: Overflow,
    /// 与队列共生命周期的 O(1) 指标上下文。
    metrics: SpawnerMetrics,
}

impl<P: Send + 'static> BoundedSpawner<P> {
    /// 构造：concurrency=N 个 worker；queue_cap=K；run = 每条 job 的 I/O 执行体
    /// （async 闭包，返回 `PortOutcome`，仅在 job.corr=Some 时回报）。
    ///
    /// N 个 worker 共享单队列（竞争 recv）；Persist 用 N=1 保写序。
    pub fn new<F, Fut>(
        concurrency: usize,
        queue_cap: usize,
        overflow: Overflow,
        reply_tx: mpsc::UnboundedSender<Tick>,
        run: F,
    ) -> Self
    where
        F: Fn(Option<Correlation>, P) -> Fut + Clone + Send + 'static,
        Fut: std::future::Future<Output = PortOutcome> + Send + 'static,
    {
        Self::new_observed(
            concurrency,
            queue_cap,
            overflow,
            reply_tx,
            Arc::new(NoopMetricSink),
            "unobserved",
            run,
        )
    }

    /// 构造带指标的工作池，并在公开 seam 上保持与 `new` 相同的执行语义。
    #[allow(clippy::too_many_arguments)]
    pub fn new_observed<F, Fut>(
        concurrency: usize,
        queue_cap: usize,
        overflow: Overflow,
        reply_tx: mpsc::UnboundedSender<Tick>,
        sink: Arc<dyn AsyncMetricSink>,
        pool: &'static str,
        run: F,
    ) -> Self
    where
        F: Fn(Option<Correlation>, P) -> Fut + Clone + Send + 'static,
        Fut: std::future::Future<Output = PortOutcome> + Send + 'static,
    {
        Self::new_with_feedback(
            concurrency,
            queue_cap,
            overflow,
            FeedbackSink::Raw(reply_tx),
            sink,
            pool,
            run,
        )
    }

    /// 汇合 raw 测试兼容入口与生产 stamped 回灌入口的唯一 worker 构造。
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new_with_feedback<F, Fut>(
        concurrency: usize,
        queue_cap: usize,
        overflow: Overflow,
        feedback_tx: FeedbackSink,
        sink: Arc<dyn AsyncMetricSink>,
        pool: &'static str,
        run: F,
    ) -> Self
    where
        F: Fn(Option<Correlation>, P) -> Fut + Clone + Send + 'static,
        Fut: std::future::Future<Output = PortOutcome> + Send + 'static,
    {
        let queue_cap = queue_cap.max(1);
        let concurrency = concurrency.max(1);
        let (tx, rx) = mpsc::channel::<QueuedJob<P>>(queue_cap);
        let metrics = SpawnerMetrics::new(sink, pool, overflow, queue_cap, concurrency);
        // N worker 共享单队列：Arc<Mutex<rx>>，竞争 recv（公平由 tokio Mutex 保证 FIFO-ish）。
        let rx = Arc::new(Mutex::new(rx));
        let joins = (0..concurrency)
            .map(|_| {
                let rx = Arc::clone(&rx);
                let feedback_tx = feedback_tx.clone();
                let run = run.clone();
                let metrics = metrics.clone();
                tokio::spawn(async move {
                    loop {
                        // 独立 worker 持续抽水。**锁有意跨 recv().await 持有**：tokio mpsc
                        // Receiver 是单消费者（recv 要 &mut self），N worker 共享须互斥。空队列时
                        // 仅一个 worker 持锁 park 在 recv，新 job 唤醒它即出块 drop 锁、下一 worker 接力
                        // （经 mutex 串行交接，非瞬时 N 名额）。真并发在锁外的 run().await(下)，故
                        // N 路 I/O 不受队列锁串行化影响——死锁安全见 AGENTS.md「Block 模式」节。
                        let job = {
                            let mut guard = rx.lock().await;
                            guard.recv().await
                        };
                        let Some(QueuedJob {
                            job: Job { corr, payload },
                            enqueued_at,
                        }) = job
                        else {
                            break;
                        };
                        metrics.on_dequeue(enqueued_at);
                        let execution_started = metrics.enabled.then(Instant::now);
                        metrics.on_execution_start();
                        let outcome = run(corr, payload).await; // 执行 I/O（不持泵锁、不持队列锁）
                        metrics.on_execution_complete(execution_started, &outcome);
                        if let Some(corr) = corr {
                            // unbounded send：不会阻塞，回灌必达（不变量1）。
                            feedback_tx.send(Tick::PortReply { corr, outcome });
                        }
                        // corr=None → fire-and-forget，不回报。
                    }
                })
            })
            .collect();
        Self {
            tx,
            joins,
            overflow,
            metrics,
        }
    }

    /// 入队一条 job。
    /// - Block：`send().await`，满则阻塞泵（背压传导回 tick_tx）。
    /// - DropNewest：try_send，满则丢**本条（最新）** + warn（泵不阻塞）。
    pub async fn submit(&self, job: Job<P>) {
        let submit_started = self.metrics.enabled.then(Instant::now);
        match self.overflow {
            Overflow::Block => {
                // 队满阻塞——独立 worker 持续抽水会腾位，泵解开（死锁安全见 struct 文档）。
                match self.tx.reserve().await {
                    Ok(permit) => {
                        self.metrics.on_enqueue(submit_started);
                        permit.send(QueuedJob {
                            job,
                            enqueued_at: self.metrics.enabled.then(Instant::now),
                        });
                    }
                    Err(_) => self.metrics.on_closed(),
                }
            }
            Overflow::DropNewest => {
                // DropNewest：try_send 满时丢本条（最新来的请求），队列里已入队的条目继续执行。
                // 在 fire-and-forget 自愈场景下（靠 cursor-gate 重发补），此行为等价于任意丢弃策略。
                // 取最简实现（try_send-Full-drop，丢本条），warn 即可（YAGNI，不引 worker 端 skip）。
                match self.tx.try_reserve() {
                    Ok(permit) => {
                        self.metrics.on_enqueue(submit_started);
                        permit.send(QueuedJob {
                            job,
                            enqueued_at: self.metrics.enabled.then(Instant::now),
                        });
                    }
                    Err(mpsc::error::TrySendError::Full(())) => {
                        self.metrics.on_drop();
                        tracing::warn!(
                            "HttpFire 队列满，丢弃本条（满即丢当前，fire-and-forget 自愈请求，靠 cursor-gate 重发兜底）"
                        );
                    }
                    Err(mpsc::error::TrySendError::Closed(())) => self.metrics.on_closed(),
                }
            }
        }
    }

    /// graceful drain：drop tx → worker recv None 退出 → join 全部。
    pub async fn shutdown(self) {
        let Self {
            tx, joins, metrics, ..
        } = self;
        drop(tx);
        for join in joins {
            if join.await.is_err_and(|error| error.is_panic()) {
                metrics.on_worker_panic();
            }
        }
    }

    /// 有上限 graceful drain（fix/lifecycle-net-decouple）：drop tx → join，全程封顶 `limit`。
    /// 限内全 join 完成 → `true`；超时 → **abort 残留 worker**（放弃在途 job）+ `false`。
    ///
    /// 为何需上限：必达 Http reqwest 单条可卡满网络 timeout（连不上 ~30s），无上限会让
    /// `helix_destroy` graceful drain 等满。**不丢保证**由乐观态持久化(status=Sending) + 重连重发
    /// 对账承担，不由「drain 必等满网络」承担——abort 在途 Http 不丢数据（消息仍 status=Sending
    /// 留库，靠重连兜底）。完整语义见 `engine.rs` 五不变量⑤ + driver AGENTS.md。
    pub async fn shutdown_with_timeout(self, limit: std::time::Duration) -> bool {
        let Self {
            tx, joins, metrics, ..
        } = self;
        drop(tx); // 关闭入队端 → worker recv None 后自然退出
                  // 先留存 abort handle（超时后 abort 残留 worker 用），再消费 JoinHandle 顺序 join。
        let abort_handles: Vec<_> = joins.iter().map(|j| j.abort_handle()).collect();
        let join_fut = async move {
            for join in joins {
                if join.await.is_err_and(|error| error.is_panic()) {
                    metrics.on_worker_panic();
                }
            }
        };
        match tokio::time::timeout(limit, join_fut).await {
            Ok(()) => true, // 限内全部 worker 已退出（积压 job 全落地）
            Err(_) => {
                // 超时：abort 残留 worker（放弃在途 job，如卡 reqwest timeout 的必达 Http）。
                for h in abort_handles {
                    h.abort();
                }
                false
            }
        }
    }
}

impl FeedbackSender {
    /// 把 PortReply/PortProgress 作为同元素 stamped Tick 回灌给 Engine。
    pub(crate) fn send(&self, tick: Tick) {
        self.tx
            .send(StampedFeedback {
                tick,
                enqueued_at: Instant::now(),
            })
            .ok();
    }
}

impl FeedbackSink {
    /// 保持 raw 测试兼容，同时让生产回灌携带真实入队时刻。
    pub(crate) fn send(&self, tick: Tick) {
        match self {
            Self::Raw(tx) => {
                tx.send(tick).ok();
            }
            Self::Stamped(tx) => tx.send(tick),
        }
    }
}

impl Overflow {
    /// 返回稳定的低基数标签，避免调试字符串进入 Prometheus series。
    const fn as_metric_label(self) -> &'static str {
        match self {
            Self::Block => "block",
            Self::DropNewest => "drop_newest",
        }
    }
}

impl SpawnerMetrics {
    /// 初始化池级原子计数，并发布固定容量与 worker 数量。
    fn new(
        sink: Arc<dyn AsyncMetricSink>,
        pool: &'static str,
        overflow: Overflow,
        queue_cap: usize,
        workers: usize,
    ) -> Self {
        let enabled = sink.is_enabled();
        let metrics = Self {
            sink,
            pool,
            overflow: overflow.as_metric_label(),
            depth: Arc::new(AtomicUsize::new(0)),
            inflight: Arc::new(AtomicUsize::new(0)),
            enabled,
        };
        metrics.gauge(MetricId::PoolQueueCapacity, queue_cap);
        metrics.gauge(MetricId::PoolWorkers, workers);
        metrics
    }

    /// 记录成功入队以及 submit 因容量产生的阻塞时间。
    fn on_enqueue(&self, started: Option<Instant>) {
        if !self.enabled {
            return;
        }
        if let Some(started) = started {
            self.histogram(MetricId::PoolEnqueueBlockSeconds, started.elapsed());
        }
        let depth = self.depth.fetch_add(1, Ordering::Relaxed) + 1;
        self.gauge(MetricId::PoolQueueDepth, depth);
    }

    /// 在 worker 取出任务时闭合真实队列驻留时间并减少 depth。
    fn on_dequeue(&self, enqueued_at: Option<Instant>) {
        if !self.enabled {
            return;
        }
        let depth = decrement_saturating(&self.depth);
        self.gauge(MetricId::PoolQueueDepth, depth);
        if let Some(enqueued_at) = enqueued_at {
            self.histogram(MetricId::PoolQueueResidencySeconds, enqueued_at.elapsed());
        }
    }

    /// 标记 worker 进入实际 I/O 执行区间。
    fn on_execution_start(&self) {
        if !self.enabled {
            return;
        }
        let inflight = self.inflight.fetch_add(1, Ordering::Relaxed) + 1;
        self.gauge(MetricId::PoolInflight, inflight);
    }

    /// 闭合 worker 执行耗时并按 PortOutcome 分类状态。
    fn on_execution_complete(&self, started: Option<Instant>, outcome: &PortOutcome) {
        if !self.enabled {
            return;
        }
        let inflight = decrement_saturating(&self.inflight);
        self.gauge(MetricId::PoolInflight, inflight);
        let Some(started) = started else {
            return;
        };
        let status = if matches!(outcome, PortOutcome::Ok(_)) {
            "ok"
        } else {
            "error"
        };
        self.record(MetricEvent::histogram(
            MetricId::PoolExecutionSeconds,
            started.elapsed().as_secs_f64(),
            self.labels().with(LabelKey::Status, status),
        ));
    }

    /// 记录 DropNewest 明确丢弃，禁止把溢出只留在日志。
    fn on_drop(&self) {
        self.counter(MetricId::PoolDroppedTotal);
    }

    /// 记录关闭后的提交，供生命周期排障使用。
    fn on_closed(&self) {
        self.counter(MetricId::PoolClosedTotal);
    }

    /// 只把真实 panic 计入 worker panic，不把主动 abort 混为故障。
    fn on_worker_panic(&self) {
        self.counter(MetricId::PoolJobPanicsTotal);
    }

    /// 构造固定 pool/overflow 标签集合。
    fn labels(&self) -> MetricLabels {
        MetricLabels::one(LabelKey::Stage, "effect")
            .with(LabelKey::Pool, self.pool)
            .with(LabelKey::Overflow, self.overflow)
    }

    /// 发布池级 Gauge 快照。
    fn gauge(&self, id: MetricId, value: usize) {
        self.record(MetricEvent::gauge(id, value as f64, self.labels()));
    }

    /// 发布池级 Counter 增量。
    fn counter(&self, id: MetricId) {
        self.record(MetricEvent::counter(id, 1.0, self.labels()));
    }

    /// 发布池级耗时直方图。
    fn histogram(&self, id: MetricId, value: std::time::Duration) {
        self.record(MetricEvent::histogram(
            id,
            value.as_secs_f64(),
            self.labels(),
        ));
    }

    /// 将指标写入有界 sink；禁用时保持纯 no-op。
    fn record(&self, event: MetricEvent) {
        if self.enabled {
            let _ = self.sink.try_record(event);
        }
    }
}

/// 原子饱和递减，避免异常关闭路径把 Gauge 下溢成 usize::MAX。
fn decrement_saturating(value: &AtomicUsize) -> usize {
    value
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
            Some(current.saturating_sub(1))
        })
        .unwrap_or_default()
        .saturating_sub(1)
}

#[cfg(test)]
#[path = "spawner_tests.rs"]
mod tests;
