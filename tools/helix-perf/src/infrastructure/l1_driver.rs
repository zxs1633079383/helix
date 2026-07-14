use std::collections::BTreeMap;
use std::time::Instant;

use bytes::Bytes;
use hdrhistogram::Histogram;
use helix_core::tick::InboundBytes;
use helix_core::{ExecutionShell, Tick};
use helix_im::module::ImConfig;
use helix_im::state::ChannelId;
use helix_im::ImModule;
use sha2::{Digest, Sha256};

use super::hdr::encode_histograms;
use super::l1_metrics::{export_l1_histograms, l1_metric_points, zero_correctness};
use super::l1_oracle::canonical_effect_bytes;
use crate::domain::{MetricPoint, RunArtifacts, RunSummary, SampleStats, ScenarioConfig, Verdict};

pub fn execute_l1(
    config: &ScenarioConfig,
    run_id: &str,
) -> Result<(RunSummary, Vec<MetricPoint>, RunArtifacts), String> {
    let windows = config.windows_ms()?;
    let warmup_iterations = ((config.run.rate * windows[0]) / 1_000).max(10);
    let load_iterations = ((config.run.rate * windows[1]) / 1_000).max(100);
    let mut shell = make_shell()?;
    for sequence in 1..=warmup_iterations {
        shell
            .step(inbound_tick(sequence, &config.run.payload_bucket), sequence)
            .map_err(|error| format!("PERF_LOAD_ABORTED: L1 warmup: {error}"))?;
    }

    let mut latency = Histogram::<u64>::new(3).map_err(|error| error.to_string())?;
    let mut effects_hist = Histogram::<u64>::new(3).map_err(|error| error.to_string())?;
    let mut bytes_hist = Histogram::<u64>::new(3).map_err(|error| error.to_string())?;
    let mut total_effects = 0_u64;
    let mut effect_mismatch = 0_u64;
    let mut trace = Vec::new();
    let load_start = Instant::now();
    for offset in 1..=load_iterations {
        let sequence = warmup_iterations + offset;
        let tick = inbound_tick(sequence, &config.run.payload_bucket);
        let started = Instant::now();
        let effects = shell
            .step(tick, sequence)
            .map_err(|error| format!("PERF_LOAD_ABORTED: L1 load: {error}"))?;
        let elapsed_us = started.elapsed().as_micros().max(1) as u64;
        let effect_count = effects.len() as u64;
        let canonical = canonical_effect_bytes(effects, sequence);
        let effect_bytes = canonical.as_ref().map_or(0, |value| value.len() as u64);
        latency
            .record(elapsed_us)
            .map_err(|error| error.to_string())?;
        effects_hist
            .record(effect_count.max(1))
            .map_err(|error| error.to_string())?;
        bytes_hist
            .record(effect_bytes.max(1))
            .map_err(|error| error.to_string())?;
        total_effects += effect_count;
        if canonical.is_err() {
            effect_mismatch += 1;
        }
        trace.push(serde_json::json!({
            "seq": sequence, "stage": "core.step", "effects": effect_count,
            "canonical": canonical.is_ok(), "error": canonical.err()
        }));
    }
    let load_elapsed = load_start.elapsed().as_secs_f64();

    let deterministic = deterministic_probe(256, &config.run.payload_bucket)?;
    let lost = u64::from(total_effects == 0);
    let mut correctness = zero_correctness();
    correctness.insert("helix_lost_total".into(), lost);
    correctness.insert(
        "helix_projection_mismatch_total".into(),
        u64::from(!deterministic).saturating_add(effect_mismatch),
    );
    let correct = correctness.values().all(|value| *value == 0);

    let stats = SampleStats::from_histogram(&latency, load_elapsed);
    let mut latency_map = BTreeMap::new();
    latency_map.insert("core_step".into(), stats.clone());
    let mut qps = BTreeMap::new();
    qps.insert("core".into(), stats.operations_per_second);
    qps.insert(
        "effect".into(),
        total_effects as f64 / load_elapsed.max(0.000_001),
    );
    let mut assertions = BTreeMap::new();
    assertions.insert("deterministic_effect_digest".into(), deterministic);
    assertions.insert("correctness_zero".into(), correct);
    assertions.insert("effects_present".into(), total_effects > 0);

    let summary = RunSummary {
        schema_version: 1,
        run_id: run_id.to_string(),
        scenario_id: config.run.scenario.clone(),
        verdict: if correct && deterministic {
            Verdict::Pass
        } else {
            Verdict::Fail
        },
        latency: latency_map,
        qps,
        rps: BTreeMap::new(),
        error_count: lost,
        error_rate: lost as f64 / load_elapsed.max(0.000_001),
        error_ratio: lost as f64 / load_iterations as f64,
        dropped: 0,
        correctness,
        assertions,
        errors: if correct {
            Vec::new()
        } else {
            vec!["PERF_CORRECTNESS_VIOLATION".into()]
        },
    };
    let metrics = l1_metric_points(
        config,
        run_id,
        &stats,
        total_effects,
        load_iterations,
        &effects_hist,
        &bytes_hist,
    );
    export_l1_histograms(
        config,
        load_iterations,
        lost,
        &latency,
        &effects_hist,
        &bytes_hist,
    )?;
    let histograms = BTreeMap::from([
        ("core_step".into(), latency),
        ("effects_per_tick".into(), effects_hist),
        ("effect_bytes_per_tick".into(), bytes_hist),
    ]);
    let artifacts = RunArtifacts {
        hdr: encode_histograms(&histograms)?,
        trace_index: trace,
        sqlite: serde_json::json!({"applicable": false, "reason": "L1 pure core"}),
        cleanup: serde_json::json!({"residue": 0, "status": "clean", "resources": []}),
        profile: serde_json::json!({"applicable": false, "reason": "L1 保持纯 core 测量"}),
    };
    Ok((summary, metrics, artifacts))
}

fn make_shell() -> Result<ExecutionShell, String> {
    let mut module = ImModule::new(ImConfig::default());
    module.register_channel(
        ChannelId::from_str("00000000000000000000000001")
            .ok_or("PERF_CONFIG_INVALID: invalid channel fixture")?,
        0,
    );
    let mut shell = ExecutionShell::new();
    shell.register(module);
    shell
        .start()
        .map_err(|error| format!("PERF_LOAD_ABORTED: core start: {error}"))?;
    Ok(shell)
}

fn inbound_tick(sequence: u64, payload_bucket: &str) -> Tick {
    let payload_len = match payload_bucket {
        "256B" => 96,
        "16KiB" => 16_000,
        _ => 1_900,
    };
    let payload = "x".repeat(payload_len);
    let frame = serde_json::json!({
        "event": "posted",
        "data": {
            "channel_id": "00000000000000000000000001",
            "seq": sequence,
            "event_type": 1,
            "msg_id": format!("perf-{sequence}"),
            "temporary_id": format!("perf-{sequence}"),
            "id": format!("perf-{sequence}"),
            "user_id": "00000000000000000000000002",
            "actor_id": "00000000000000000000000002",
            "type": "TEXT",
            "create_at": 1_000_000_u64 + sequence,
            "message": payload
        }
    });
    let bytes = serde_json::to_vec(&frame).unwrap_or_default();
    Tick::Inbound(InboundBytes(Bytes::from(bytes)))
}

fn deterministic_probe(iterations: u64, payload_bucket: &str) -> Result<bool, String> {
    fn digest(iterations: u64, payload_bucket: &str) -> Result<Vec<u8>, String> {
        let mut shell = make_shell()?;
        let mut hash = Sha256::new();
        for sequence in 1..=iterations {
            let effects = shell
                .step(inbound_tick(sequence, payload_bucket), sequence)
                .map_err(|error| error.to_string())?;
            hash.update(canonical_effect_bytes(effects, sequence)?);
        }
        Ok(hash.finalize().to_vec())
    }
    Ok(digest(iterations, payload_bucket)? == digest(iterations, payload_bucket)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_oracle_checks_real_effect_order_and_sequence_payload() {
        let mut shell = make_shell().expect("shell");
        let effects = shell.step(inbound_tick(1, "256B"), 1).expect("step");
        assert!(!canonical_effect_bytes(effects, 1)
            .expect("canonical")
            .is_empty());
        assert!(canonical_effect_bytes(effects, 2).is_err());
        assert!(deterministic_probe(8, "256B").expect("digest"));
    }
}
