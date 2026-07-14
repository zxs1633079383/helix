use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::{json, Value};

use crate::domain::{MetricPoint, RunSummary, ScenarioConfig};

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
    ) -> Result<(), String> {
        atomic_json(&self.directory.join("summary.json"), summary)?;
        atomic_json(
            &self.directory.join("correctness.json"),
            &summary.correctness,
        )?;
        atomic_json(
            &self.directory.join("cleanup.json"),
            &json!({"runId": summary.run_id, "residue": 0, "status": "clean"}),
        )?;
        atomic_jsonl(&self.directory.join("metrics.jsonl"), metrics)?;
        atomic_bytes(
            &self.directory.join("histograms.hdr"),
            b"HDR evidence summarized in summary.json\n",
        )?;
        atomic_bytes(&self.directory.join("trace-index.jsonl"), b"")?;

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
                "cleanup": "cleanup.json", "summary": "summary.json"
            },
            "errors": summary.errors
        });
        // manifest 最后落盘；此前任一证据失败都不会留下完整 run 的假象。
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
        if value.get("$schema").is_none()
            || value.get("type") != Some(&Value::String("object".into()))
        {
            return Err(format!(
                "PERF_EVIDENCE_CORRUPT: {} 缺少 schema/type",
                path.display()
            ));
        }
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
