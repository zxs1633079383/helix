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

use std::sync::Arc;

use tokio::sync::{mpsc, Mutex};
use tokio::task::JoinHandle;

use helix_core::effect::Correlation;
use helix_core::tick::PortOutcome;
use helix_core::Tick;

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

/// 通用有界并发原语：有界 mpsc(cap=K) + N 个常驻 worker + 溢出策略。
pub struct BoundedSpawner<P: Send + 'static> {
    /// 有界 job 发送端（cap = queue_cap）。
    tx: mpsc::Sender<Job<P>>,
    /// N 个 worker task 句柄，shutdown 时 join 确保积压（必达类）全部落地。
    joins: Vec<JoinHandle<()>>,
    /// 溢出策略。
    overflow: Overflow,
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
        F: Fn(P) -> Fut + Clone + Send + 'static,
        Fut: std::future::Future<Output = PortOutcome> + Send + 'static,
    {
        let (tx, rx) = mpsc::channel::<Job<P>>(queue_cap.max(1));
        // N worker 共享单队列：Arc<Mutex<rx>>，竞争 recv（公平由 tokio Mutex 保证 FIFO-ish）。
        let rx = Arc::new(Mutex::new(rx));
        let joins = (0..concurrency.max(1))
            .map(|_| {
                let rx = Arc::clone(&rx);
                let reply_tx = reply_tx.clone();
                let run = run.clone();
                tokio::spawn(async move {
                    loop {
                        // 独立 worker 持续抽水。**锁有意跨 recv().await 持有**：tokio mpsc
                        // Receiver 是单消费者（recv 要 &mut self），N worker 共享须互斥。空队列时
                        // 仅一个 worker 持锁 park 在 recv，新 job 唤醒它即出块 drop 锁、下一 worker 接力
                        // （经 mutex 串行交接，非瞬时 N 名额）。真并发在锁外的 run().await(下)，故
                        // N 路 I/O 不受队列锁串行化影响——死锁安全见 CLAUDE.md「Block 模式」节。
                        let job = {
                            let mut guard = rx.lock().await;
                            guard.recv().await
                        };
                        let Some(Job { corr, payload }) = job else {
                            break;
                        };
                        let outcome = run(payload).await; // 执行 I/O（不持泵锁、不持队列锁）
                        if let Some(corr) = corr {
                            // unbounded send：不会阻塞，回灌必达（不变量1）。
                            reply_tx.send(Tick::PortReply { corr, outcome }).ok();
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
        }
    }

    /// 入队一条 job。
    /// - Block：`send().await`，满则阻塞泵（背压传导回 tick_tx）。
    /// - DropNewest：try_send，满则丢**本条（最新）** + warn（泵不阻塞）。
    pub async fn submit(&self, job: Job<P>) {
        match self.overflow {
            Overflow::Block => {
                // 队满阻塞——独立 worker 持续抽水会腾位，泵解开（死锁安全见 struct 文档）。
                self.tx.send(job).await.ok();
            }
            Overflow::DropNewest => {
                // DropNewest：try_send 满时丢本条（最新来的请求），队列里已入队的条目继续执行。
                // 在 fire-and-forget 自愈场景下（靠 cursor-gate 重发补），此行为等价于任意丢弃策略。
                // 取最简实现（try_send-Full-drop，丢本条），warn 即可（YAGNI，不引 worker 端 skip）。
                if self.tx.try_send(job).is_err() {
                    tracing::warn!(
                        "HttpFire 队列满，丢弃本条（满即丢当前，fire-and-forget 自愈请求，靠 cursor-gate 重发兜底）"
                    );
                }
            }
        }
    }

    /// graceful drain：drop tx → worker recv None 退出 → join 全部。
    pub async fn shutdown(self) {
        drop(self.tx);
        for j in self.joins {
            let _ = j.await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use helix_core::tick::ReplyBytes;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;

    /// ① HttpFire 满队列 DropNewest：泵不阻塞，溢出（最新来的）条目被丢，已入队的最终执行。
    #[tokio::test]
    async fn test_bounded_spawner_drop_newest_no_block() {
        let executed = Arc::new(AtomicU64::new(0));
        let gate = Arc::new(tokio::sync::Notify::new());
        let (reply_tx, _reply_rx) = mpsc::unbounded_channel::<Tick>();

        let executed_c = Arc::clone(&executed);
        let gate_c = Arc::clone(&gate);
        // N=1，queue_cap=1，worker 在第一条 job 内 park 在 gate 上 → 队列很快填满。
        let sp: BoundedSpawner<u64> = BoundedSpawner::new(
            1,
            1,
            Overflow::DropNewest,
            reply_tx,
            move |_payload: u64| {
                let executed = Arc::clone(&executed_c);
                let gate = Arc::clone(&gate_c);
                async move {
                    gate.notified().await; // 卡住 worker，制造队满
                    executed.fetch_add(1, Ordering::SeqCst);
                    PortOutcome::Ok(ReplyBytes::default())
                }
            },
        );

        // 提交 100 条：DropNewest 下 submit 永不阻塞（worker 卡住，队满即丢本条）。
        let submit_all = async {
            for i in 0..100u64 {
                sp.submit(Job {
                    corr: None,
                    payload: i,
                })
                .await;
            }
        };
        tokio::time::timeout(Duration::from_secs(2), submit_all)
            .await
            .expect("DropNewest submit 必须不阻塞泵（未超时）");

        // 放行 worker，等待消化。queue_cap=1 + worker 卡 1 条 → 大量被丢。
        for _ in 0..200 {
            gate.notify_one();
        }
        sp.shutdown().await;

        let done = executed.load(Ordering::SeqCst);
        assert!(
            done < 100,
            "DropNewest 必须丢弃溢出条目（最新来的），实际执行 {done}/100（应远小于 100）"
        );
    }

    /// ② Http 满队列 Block：泵 submit 阻塞，worker 抽水后解开，零丢失（全部执行）。
    #[tokio::test]
    async fn test_bounded_spawner_block_no_loss() {
        let executed = Arc::new(AtomicU64::new(0));
        let (reply_tx, _reply_rx) = mpsc::unbounded_channel::<Tick>();

        let executed_c = Arc::clone(&executed);
        let sp: BoundedSpawner<u64> =
            BoundedSpawner::new(1, 1, Overflow::Block, reply_tx, move |_payload: u64| {
                let executed = Arc::clone(&executed_c);
                async move {
                    tokio::time::sleep(Duration::from_millis(1)).await;
                    executed.fetch_add(1, Ordering::SeqCst);
                    PortOutcome::Ok(ReplyBytes::default())
                }
            });

        const N: u64 = 50;
        for i in 0..N {
            sp.submit(Job {
                corr: None,
                payload: i,
            })
            .await;
        }
        sp.shutdown().await;

        assert_eq!(
            executed.load(Ordering::SeqCst),
            N,
            "Block 模式必达零丢失：{N} 条必须全部执行"
        );
    }

    /// ③ Persist 写序：N=1 单 worker 单消费者，入队序 == 执行序（不变量3）。
    #[tokio::test]
    async fn test_bounded_spawner_n1_preserves_order() {
        let order = Arc::new(Mutex::new(Vec::<u64>::new()));
        let (reply_tx, _reply_rx) = mpsc::unbounded_channel::<Tick>();

        let order_c = Arc::clone(&order);
        let sp: BoundedSpawner<u64> = BoundedSpawner::new(
            1, // N=1 保序
            8,
            Overflow::Block,
            reply_tx,
            move |payload: u64| {
                let order = Arc::clone(&order_c);
                async move {
                    // 故意让靠前的条目「执行体」更慢，若并发(N>1)会乱序——N=1 必有序。
                    tokio::time::sleep(Duration::from_millis(10 - (payload % 10))).await;
                    order.lock().await.push(payload);
                    PortOutcome::Ok(ReplyBytes::default())
                }
            },
        );

        for i in 0..20u64 {
            sp.submit(Job {
                corr: None,
                payload: i,
            })
            .await;
        }
        sp.shutdown().await;

        let recorded = order.lock().await.clone();
        let expected: Vec<u64> = (0..20).collect();
        assert_eq!(
            recorded, expected,
            "N=1 BoundedSpawner 必须保持入队序 == 执行序"
        );
    }

    /// ②附：Block 模式带 corr 回报 → reply_tx 收到 PortReply（回灌必达，不变量1）。
    #[tokio::test]
    async fn test_bounded_spawner_reports_via_reply_tx() {
        let (reply_tx, mut reply_rx) = mpsc::unbounded_channel::<Tick>();
        let sp: BoundedSpawner<u64> = BoundedSpawner::new(
            1,
            8,
            Overflow::Block,
            reply_tx,
            move |_payload: u64| async move { PortOutcome::Ok(ReplyBytes::default()) },
        );

        sp.submit(Job {
            corr: Some(Correlation::from_raw(42)),
            payload: 1,
        })
        .await;
        sp.shutdown().await;

        let reply = reply_rx.try_recv().expect("应收到 PortReply 回灌");
        match reply {
            Tick::PortReply { corr, .. } => assert_eq!(corr.raw(), 42),
            _ => panic!("期望 Tick::PortReply"),
        }
    }
}
