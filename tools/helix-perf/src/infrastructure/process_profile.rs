use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::Serialize;

pub struct CountingAllocator;

static ALLOC_COUNT: AtomicU64 = AtomicU64::new(0);
static ALLOC_BYTES: AtomicU64 = AtomicU64::new(0);

// SAFETY: 所有分配与释放仍逐字委托给 `System`；这里只做 relaxed 原子旁路计数，
// 不保存或改写任何指针，也不参与分配器所有权。
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
        ALLOC_BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
        // SAFETY: 调用者把同一个有效 layout 交给全局分配器，原样转交 System。
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: ptr/layout 来自对应的 System 分配，原样转交释放。
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
        ALLOC_BYTES.fetch_add(new_size as u64, Ordering::Relaxed);
        // SAFETY: ptr/layout 来自 System，new_size 遵守 GlobalAlloc 契约。
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ProcessSnapshot {
    allocations: u64,
    allocated_bytes: u64,
    cpu_seconds: f64,
    peak_rss_bytes: u64,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessProfile {
    pub allocations: u64,
    pub allocated_bytes: u64,
    pub allocations_per_operation: f64,
    pub allocated_bytes_per_operation: f64,
    pub cpu_seconds: f64,
    pub cpu_seconds_per_operation: f64,
    pub peak_rss_bytes: u64,
}

pub fn snapshot() -> ProcessSnapshot {
    let (cpu_seconds, peak_rss_bytes) = resource_usage();
    ProcessSnapshot {
        allocations: ALLOC_COUNT.load(Ordering::Relaxed),
        allocated_bytes: ALLOC_BYTES.load(Ordering::Relaxed),
        cpu_seconds,
        peak_rss_bytes,
    }
}

pub fn delta(start: ProcessSnapshot, operations: u64) -> ProcessProfile {
    let end = snapshot();
    let divisor = operations.max(1) as f64;
    let allocations = end.allocations.saturating_sub(start.allocations);
    let allocated_bytes = end.allocated_bytes.saturating_sub(start.allocated_bytes);
    let cpu_seconds = (end.cpu_seconds - start.cpu_seconds).max(0.0);
    ProcessProfile {
        allocations,
        allocated_bytes,
        allocations_per_operation: allocations as f64 / divisor,
        allocated_bytes_per_operation: allocated_bytes as f64 / divisor,
        cpu_seconds,
        cpu_seconds_per_operation: cpu_seconds / divisor,
        peak_rss_bytes: end.peak_rss_bytes.max(start.peak_rss_bytes),
    }
}

#[cfg(feature = "sdk-metrics")]
pub fn export_metrics(
    sink: &dyn helix_driver_host::metrics::AsyncMetricSink,
    profile: &ProcessProfile,
) -> Result<(), String> {
    use helix_driver_host::metrics::{
        LabelKey, MetricEvent, MetricId, MetricLabels, RecordOutcome,
    };

    if !sink.is_enabled() {
        return Ok(());
    }
    let labels = MetricLabels::one(LabelKey::Layer, "L2")
        .with(LabelKey::Stage, "telemetry")
        .with(LabelKey::Operation, "rated_profile")
        .with(LabelKey::Status, "ok");
    for event in [
        MetricEvent::histogram(
            MetricId::AllocCountPerOp,
            profile.allocations_per_operation,
            labels,
        ),
        MetricEvent::histogram(
            MetricId::AllocBytesPerOp,
            profile.allocated_bytes_per_operation,
            labels,
        ),
        MetricEvent::gauge(
            MetricId::ProcessResidentMemoryBytes,
            profile.peak_rss_bytes as f64,
            labels,
        ),
    ] {
        if !matches!(sink.try_record(event), RecordOutcome::Accepted) {
            return Err("PERF_TELEMETRY_UNAVAILABLE: profile 指标入队失败".into());
        }
    }
    Ok(())
}

fn resource_usage() -> (f64, u64) {
    let pid = std::process::id().to_string();
    let output = std::process::Command::new("ps")
        .args(["-o", "time=", "-o", "rss=", "-p", &pid])
        .output();
    let Ok(output) = output else { return (0.0, 0) };
    if !output.status.success() {
        return (0.0, 0);
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let mut fields = text.split_whitespace();
    let cpu_seconds = fields.next().and_then(parse_cpu_time).unwrap_or(0.0);
    let rss_bytes = fields
        .next()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0)
        .saturating_mul(1_024);
    (cpu_seconds, rss_bytes)
}

fn parse_cpu_time(value: &str) -> Option<f64> {
    let fields = value.split(':').collect::<Vec<_>>();
    let (hours, minutes, seconds) = match fields.as_slice() {
        [minutes, seconds] => (
            0.0,
            minutes.parse::<f64>().ok()?,
            seconds.parse::<f64>().ok()?,
        ),
        [hours, minutes, seconds] => (
            hours.parse::<f64>().ok()?,
            minutes.parse::<f64>().ok()?,
            seconds.parse::<f64>().ok()?,
        ),
        _ => return None,
    };
    Some(hours * 3_600.0 + minutes * 60.0 + seconds)
}
