//! BoundedSpawner 单元测试（从 spawner.rs 抽出，src ≤300 行硬顶；测试软目标）。
//!
//! 经 `#[cfg(test)] #[path="spawner_tests.rs"] mod tests;` 挂回 spawner.rs，`use super::*` 访
//! 私有构造。覆盖：DropNewest 不阻塞 / Block 零丢失 / N=1 保序 / reply_tx 回灌必达。

use super::*;
use crate::metrics::{MetricEvent, MetricId, RecordOutcome};
use helix_core::tick::ReplyBytes;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex as StdMutex;
use std::time::{Duration, Instant};

#[derive(Default)]
struct RecordingMetricSink(StdMutex<Vec<MetricEvent>>);

impl AsyncMetricSink for RecordingMetricSink {
    /// 保存 worker 对外可观察事件，测试不进入真实 exporter。
    fn try_record(&self, event: MetricEvent) -> RecordOutcome {
        self.0.lock().unwrap().push(event);
        RecordOutcome::Accepted
    }
}

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
        move |_corr, _payload: u64| {
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
    let sp: BoundedSpawner<u64> = BoundedSpawner::new(
        1,
        1,
        Overflow::Block,
        reply_tx,
        move |_corr, _payload: u64| {
            let executed = Arc::clone(&executed_c);
            async move {
                tokio::time::sleep(Duration::from_millis(1)).await;
                executed.fetch_add(1, Ordering::SeqCst);
                PortOutcome::Ok(ReplyBytes::default())
            }
        },
    );

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
        move |_corr, payload: u64| {
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

/// ④ 有上限 drain：在途 worker 卡死时 `shutdown_with_timeout` 必在 limit 内返回 false 并 abort。
///
/// 这是 `engine.rs` graceful drain「必达 Http drain 有上限」（HX-C001 五不变量⑤ / fix/lifecycle-net-decouple）
/// 的**确定性**证——不依赖真 reqwest / blackhole / 墙钟，故不受并行测试负载抖动影响（对照
/// `helix-driver-ffi/tests/lifecycle_net_decouple.rs` 的端到端墙钟界，那里只做粗 sanity）。
/// worker park 60s（远大于 limit 50ms，1200x 余量 → 永不 flake）：limit 内 join 不完 → 必走超时 abort 分支。
#[tokio::test]
async fn shutdown_with_timeout_caps_and_aborts_inflight() {
    let entered = Arc::new(AtomicU64::new(0));
    let (reply_tx, _reply_rx) = mpsc::unbounded_channel::<Tick>();

    let entered_c = Arc::clone(&entered);
    let sp: BoundedSpawner<u64> = BoundedSpawner::new(
        1,
        1,
        Overflow::Block,
        reply_tx,
        move |_corr, _payload: u64| {
            let entered = Arc::clone(&entered_c);
            async move {
                entered.fetch_add(1, Ordering::SeqCst);
                // 模拟卡满网络 timeout 的在途必达 Http（连不上 host 的 reqwest）。
                tokio::time::sleep(Duration::from_secs(60)).await;
                PortOutcome::Ok(ReplyBytes::default())
            }
        },
    );

    sp.submit(Job {
        corr: Some(Correlation::from_raw(1)),
        payload: 1,
    })
    .await;
    // worker 已进入 60s sleep 后才 drain（否则可能 join 在 sleep 前秒回，测不到超时路径）。
    for _ in 0..200 {
        if entered.load(Ordering::SeqCst) == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert_eq!(
        entered.load(Ordering::SeqCst),
        1,
        "worker 应已进入在途 sleep"
    );

    let t = Instant::now();
    let all_drained = sp.shutdown_with_timeout(Duration::from_millis(50)).await;
    let elapsed = t.elapsed();

    assert!(
        !all_drained,
        "在途 worker 卡 60s，50ms 上限必超时返回 false（abort 在途，不等满）"
    );
    // 50ms 上限 vs 60s sleep：即便重载，elapsed 也远小于 worker sleep → 证明确实 abort 而非等满。
    assert!(
        elapsed < Duration::from_secs(5),
        "shutdown_with_timeout 必在远小于 worker 60s sleep 内返回（abort 在途），实际 {elapsed:?}"
    );
}

/// ④附：在途全部 limit 内完成时，`shutdown_with_timeout` 返回 true（不误 abort 已落地 job）。
#[tokio::test]
async fn shutdown_with_timeout_returns_true_when_drained_in_limit() {
    let executed = Arc::new(AtomicU64::new(0));
    let (reply_tx, _reply_rx) = mpsc::unbounded_channel::<Tick>();

    let executed_c = Arc::clone(&executed);
    let sp: BoundedSpawner<u64> = BoundedSpawner::new(
        1,
        8,
        Overflow::Block,
        reply_tx,
        move |_corr, _payload: u64| {
            let executed = Arc::clone(&executed_c);
            async move {
                executed.fetch_add(1, Ordering::SeqCst);
                PortOutcome::Ok(ReplyBytes::default())
            }
        },
    );

    for i in 0..5u64 {
        sp.submit(Job {
            corr: None,
            payload: i,
        })
        .await;
    }
    // 5 条秒级 job，5s 上限绰绰有余 → 全 join 完成 → true。
    let all_drained = sp.shutdown_with_timeout(Duration::from_secs(5)).await;
    assert!(all_drained, "秒级 job 在 5s 上限内必全部 drain 完成 → true");
    assert_eq!(
        executed.load(Ordering::SeqCst),
        5,
        "limit 内 drain：5 条必达 job 全部执行（不误 abort）"
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
        move |_corr, _payload: u64| async move { PortOutcome::Ok(ReplyBytes::default()) },
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

/// 记录型 sink 必须看见真实 queue residency、执行区间和最终归零 Gauge。
#[tokio::test]
async fn observed_spawner_exposes_queue_and_execution_boundaries() {
    let metrics = Arc::new(RecordingMetricSink::default());
    let (reply_tx, _reply_rx) = mpsc::unbounded_channel::<Tick>();
    let sp = BoundedSpawner::new_observed(
        1,
        2,
        Overflow::Block,
        reply_tx,
        metrics.clone(),
        "persist",
        move |_corr, _payload: u64| async move { PortOutcome::Ok(ReplyBytes::default()) },
    );

    sp.submit(Job {
        corr: None,
        payload: 7,
    })
    .await;
    sp.shutdown().await;

    let events = metrics.0.lock().unwrap();
    for expected in [
        MetricId::PoolQueueCapacity,
        MetricId::PoolWorkers,
        MetricId::PoolEnqueueBlockSeconds,
        MetricId::PoolQueueResidencySeconds,
        MetricId::PoolExecutionSeconds,
        MetricId::PoolQueueDepth,
        MetricId::PoolInflight,
    ] {
        assert!(
            events.iter().any(|event| event.id == expected),
            "observed pool 缺少 {:?}",
            expected
        );
    }
    assert!(events
        .iter()
        .any(|event| { event.id == MetricId::PoolQueueDepth && event.value == 0.0 }));
    assert!(events
        .iter()
        .any(|event| { event.id == MetricId::PoolInflight && event.value == 0.0 }));
}
