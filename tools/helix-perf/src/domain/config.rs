use std::fs;
use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ScenarioConfig {
    pub schema_version: u32,
    pub run: RunConfig,
    pub windows: WindowConfig,
    pub topology: TopologyConfig,
    pub thresholds: ThresholdConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RunConfig {
    pub scenario: String,
    pub platform: String,
    pub channel_shape: String,
    pub rate: u64,
    pub rate_unit: String,
    pub payload_bucket: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WindowConfig {
    pub warmup: String,
    pub load: String,
    pub drain: String,
    pub observation: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TopologyConfig {
    pub channels: usize,
    pub senders: usize,
    pub receivers: usize,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ThresholdConfig {
    pub profile: String,
    pub max_telemetry_p99_overhead_ratio: f64,
    pub max_queue_wait_p99_ms: f64,
    pub max_drop_ratio_rated: f64,
    pub min_saturation_drops: u64,
}

impl ScenarioConfig {
    pub fn load(path: &Path) -> Result<Self, String> {
        let text = fs::read_to_string(path)
            .map_err(|error| format!("PERF_CONFIG_INVALID: {}: {error}", path.display()))?;
        let value: Self = toml::from_str(&text)
            .map_err(|error| format!("PERF_CONFIG_INVALID: {}: {error}", path.display()))?;
        value.validate()?;
        Ok(value)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != 1 || self.run.rate == 0 {
            return Err("PERF_CONFIG_INVALID: schema_version 必须为 1 且 rate > 0".into());
        }
        if !matches!(
            self.run.channel_shape.as_str(),
            "hot-1" | "parallel-50" | "fan-in-50-to-1-client" | "multi-module"
        ) {
            return Err("PERF_CONFIG_INVALID: channel_shape 非合同枚举".into());
        }
        for value in [
            &self.windows.warmup,
            &self.windows.load,
            &self.windows.drain,
            &self.windows.observation,
        ] {
            parse_duration(value)?;
        }
        Ok(())
    }

    pub fn windows_ms(&self) -> Result<[u64; 4], String> {
        Ok([
            parse_duration(&self.windows.warmup)?.as_millis() as u64,
            parse_duration(&self.windows.load)?.as_millis() as u64,
            parse_duration(&self.windows.drain)?.as_millis() as u64,
            parse_duration(&self.windows.observation)?.as_millis() as u64,
        ])
    }
}

fn parse_duration(value: &str) -> Result<Duration, String> {
    let (number, multiplier) = if let Some(raw) = value.strip_suffix("ms") {
        (raw, 1)
    } else if let Some(raw) = value.strip_suffix('s') {
        (raw, 1_000)
    } else if let Some(raw) = value.strip_suffix('m') {
        (raw, 60_000)
    } else {
        return Err(format!(
            "PERF_CONFIG_INVALID: duration 缺少 ms/s/m 单位: {value}"
        ));
    };
    let amount = number
        .parse::<u64>()
        .map_err(|_| format!("PERF_CONFIG_INVALID: duration 非整数: {value}"))?;
    Ok(Duration::from_millis(amount.saturating_mul(multiplier)))
}

#[cfg(test)]
mod tests {
    use super::parse_duration;

    #[test]
    fn duration_requires_unit() {
        assert!(parse_duration("10").is_err());
        assert_eq!(parse_duration("2s").expect("2s").as_millis(), 2_000);
    }
}
