use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::{json, Value};

use crate::domain::{MetricPoint, RunArtifacts, RunSummary, ScenarioConfig, Verdict};

pub struct EvidenceWriter {
    directory: PathBuf,
}

impl EvidenceWriter {
    pub fn create(root: &Path, run_id: &str) -> Result<Self, String> {
        let directory = root.join("perf/reports").join(run_id);
        fs::create_dir_all(&directory)
            .map_err(|error| format!("PERF_EVIDENCE_CORRUPT: {error}"))?;
        Ok(Self { directory })
    }

    pub fn write(
        &self,
        config: &ScenarioConfig,
        summary: &RunSummary,
        metrics: &[MetricPoint],
        artifacts: &RunArtifacts,
    ) -> Result<(), String> {
        let summary_value = serde_json::to_value(summary).map_err(evidence_error)?;
        atomic_json(&self.directory.join("summary.json"), &summary_value)?;
        atomic_json(
            &self.directory.join("correctness.json"),
            &summary.correctness,
        )?;
        atomic_json(&self.directory.join("cleanup.json"), &artifacts.cleanup)?;
        atomic_json(&self.directory.join("sqlite.json"), &artifacts.sqlite)?;
        atomic_json(&self.directory.join("profile.json"), &artifacts.profile)?;
        atomic_jsonl(&self.directory.join("metrics.jsonl"), metrics)?;
        atomic_bytes(&self.directory.join("histograms.hdr"), &artifacts.hdr)?;
        atomic_values_jsonl(
            &self.directory.join("trace-index.jsonl"),
            &artifacts.trace_index,
        )?;

        let windows = config.windows_ms()?;
        let manifest = json!({
            "schemaVersion": 1,
            "runId": summary.run_id,
            "scenarioId": summary.scenario_id,
            "verdict": summary.verdict,
            "git": git_snapshot(),
            "runtime": runtime_snapshot(),
            "scenario": {
                "channelShape": config.run.channel_shape,
                "rate": config.run.rate,
                "rateUnit": config.run.rate_unit,
                "payloadBucket": config.run.payload_bucket
            },
            "windows": {
                "warmupMs": windows[0], "loadMs": windows[1],
                "drainMs": windows[2], "observationMs": windows[3]
            },
            "telemetry": {
                "mode": if summary.assertions.contains_key("telemetry_overhead_within_budget") { "compared" } else { "disabled" },
                "dashboardUid": "helix-runtime-performance-pre"
            },
            "evidence": {
                "metrics": "metrics.jsonl", "histograms": "histograms.hdr",
                "traces": "trace-index.jsonl", "correctness": "correctness.json",
                "cleanup": "cleanup.json", "sqlite": "sqlite.json", "summary": "summary.json"
                , "profile": "profile.json"
            },
            "errors": summary.errors
        });
        validate_value(&manifest, &self.directory, "run", config)?;
        for metric in metrics {
            let value = serde_json::to_value(metric).map_err(evidence_error)?;
            validate_schema_value(&value, &workspace_from_directory(&self.directory), "metric")?;
        }
        // manifest 最后落盘；此前任一证据失败都不会留下完整 run 的假象。
        atomic_json(&self.directory.join("manifest.json"), &manifest)
    }

    pub fn write_failure(
        &self,
        config: &ScenarioConfig,
        run_id: &str,
        error: &str,
    ) -> Result<(), String> {
        let manifest = json!({
            "schemaVersion": 1, "runId": run_id, "scenarioId": config.run.scenario,
            "verdict": Verdict::Fail, "git": git_snapshot(), "runtime": runtime_snapshot(),
            "scenario": {"channelShape": config.run.channel_shape, "rate": config.run.rate,
                "rateUnit": config.run.rate_unit, "payloadBucket": config.run.payload_bucket},
            "windows": {"warmupMs": 0, "loadMs": 0, "drainMs": 0, "observationMs": 0},
            "telemetry": {"mode": "failed", "dashboardUid": "helix-runtime-performance-pre"},
            "evidence": {}, "errors": [error]
        });
        validate_schema_value(&manifest, &workspace_from_directory(&self.directory), "run")?;
        atomic_json(&self.directory.join("manifest.json"), &manifest)
    }
}

pub fn new_run_id(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    format!("perf-{prefix}-{nanos:x}")
}

pub fn validate_json_contracts(root: &Path) -> Result<(), String> {
    for name in ["scenario", "metric", "run"] {
        let path = root.join(format!("perf/schema/{name}.schema.json"));
        let text = fs::read_to_string(&path)
            .map_err(|error| format!("PERF_EVIDENCE_CORRUPT: {}: {error}", path.display()))?;
        let value: Value = serde_json::from_str(&text)
            .map_err(|error| format!("PERF_EVIDENCE_CORRUPT: {}: {error}", path.display()))?;
        jsonschema::validator_for(&value)
            .map_err(|error| format!("PERF_EVIDENCE_CORRUPT: {}: {error}", path.display()))?;
    }
    Ok(())
}

fn atomic_json(path: &Path, value: &impl Serialize) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(value)
        .map_err(|error| format!("PERF_EVIDENCE_CORRUPT: {error}"))?;
    atomic_bytes(path, &bytes)
}

fn atomic_jsonl(path: &Path, values: &[MetricPoint]) -> Result<(), String> {
    let tmp = path.with_extension("jsonl.tmp");
    let file = File::create(&tmp).map_err(evidence_error)?;
    let mut writer = BufWriter::new(file);
    for value in values {
        serde_json::to_writer(&mut writer, value).map_err(evidence_error)?;
        writer.write_all(b"\n").map_err(evidence_error)?;
    }
    writer.flush().map_err(evidence_error)?;
    writer.get_ref().sync_all().map_err(evidence_error)?;
    fs::rename(tmp, path).map_err(evidence_error)
}

fn atomic_values_jsonl(path: &Path, values: &[Value]) -> Result<(), String> {
    let tmp = path.with_extension("jsonl.tmp");
    let file = File::create(&tmp).map_err(evidence_error)?;
    let mut writer = BufWriter::new(file);
    for value in values {
        serde_json::to_writer(&mut writer, value).map_err(evidence_error)?;
        writer.write_all(b"\n").map_err(evidence_error)?;
    }
    writer.flush().map_err(evidence_error)?;
    writer.get_ref().sync_all().map_err(evidence_error)?;
    fs::rename(tmp, path).map_err(evidence_error)
}

fn validate_value(
    value: &Value,
    directory: &Path,
    schema: &str,
    _config: &ScenarioConfig,
) -> Result<(), String> {
    validate_schema_value(value, &workspace_from_directory(directory), schema)
}

fn validate_schema_value(value: &Value, root: &Path, schema: &str) -> Result<(), String> {
    let schema_path = root.join(format!("perf/schema/{schema}.schema.json"));
    let schema_value: Value =
        serde_json::from_slice(&fs::read(&schema_path).map_err(evidence_error)?)
            .map_err(evidence_error)?;
    let validator = jsonschema::validator_for(&schema_value).map_err(evidence_error)?;
    validator
        .validate(value)
        .map_err(|error| format!("PERF_EVIDENCE_CORRUPT: {schema}: {error}"))
}

fn workspace_from_directory(directory: &Path) -> PathBuf {
    directory
        .ancestors()
        .find(|path| path.join("Cargo.toml").exists() && path.join("perf/schema").exists())
        .unwrap_or(directory)
        .to_path_buf()
}

fn atomic_bytes(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let tmp = path.with_extension("tmp");
    let mut file = File::create(&tmp).map_err(evidence_error)?;
    file.write_all(bytes).map_err(evidence_error)?;
    file.sync_all().map_err(evidence_error)?;
    fs::rename(tmp, path).map_err(evidence_error)
}

fn evidence_error(error: impl std::fmt::Display) -> String {
    format!("PERF_EVIDENCE_CORRUPT: {error}")
}

fn git_snapshot() -> Value {
    let sha = command_output("git", &["rev-parse", "HEAD"]);
    let branch = command_output("git", &["branch", "--show-current"]);
    let dirty = !command_output("git", &["status", "--porcelain"]).is_empty();
    json!({"sha": sha, "branch": branch, "dirty": dirty})
}

fn runtime_snapshot() -> Value {
    json!({
        "rustc": command_output("rustc", &["--version"]),
        "profile": "dev",
        "platform": std::env::consts::OS,
        "arch": std::env::consts::ARCH
    })
}

fn command_output(program: &str, args: &[&str]) -> String {
    std::process::Command::new(program)
        .args(args)
        .output()
        .ok()
        .filter(|result| result.status.success())
        .map(|result| String::from_utf8_lossy(&result.stdout).trim().to_string())
        .unwrap_or_else(|| "unavailable".to_string())
}

#[cfg(test)]
mod tests {
    use super::atomic_bytes;

    #[test]
    fn atomic_write_replaces_temp_file() {
        let path = std::env::temp_dir().join("helix-perf-atomic-test.json");
        atomic_bytes(&path, b"{}\n").expect("atomic write");
        assert_eq!(std::fs::read(&path).expect("read"), b"{}\n");
        assert!(!path.with_extension("tmp").exists());
        let _ = std::fs::remove_file(path);
    }
}
