use std::path::Path;

use crate::domain::ScenarioConfig;
use crate::infrastructure::{
    execute_l1, execute_l2, new_run_id, validate_json_contracts, EvidenceWriter,
};

pub fn validate_contracts(root: &Path) -> Result<(), String> {
    validate_json_contracts(root)?;
    println!("PERF_CONTRACTS_OK: scenario/metric/run schema 可解析");
    Ok(())
}

pub async fn run_l1(root: &Path, config: ScenarioConfig) -> Result<(), String> {
    if !config.run.scenario.starts_with("L1-") {
        return Err("PERF_CONFIG_INVALID: L1 入口只能执行 L1-* 场景".into());
    }
    let run_id = new_run_id("l1");
    let (summary, metrics) = execute_l1(&config, &run_id)?;
    EvidenceWriter::create(root, &run_id)?.write(&config, &summary, &metrics)?;
    println!(
        "PERF_RUN_COMPLETE run_id={} verdict={:?} evidence=perf/reports/{}",
        run_id, summary.verdict, run_id
    );
    if matches!(summary.verdict, crate::domain::Verdict::Fail) {
        Err("PERF_CORRECTNESS_VIOLATION: L1 verdict FAIL".into())
    } else {
        Ok(())
    }
}

pub async fn run_l2(root: &Path, config: ScenarioConfig) -> Result<(), String> {
    if !config.run.scenario.starts_with("L2-") {
        return Err("PERF_CONFIG_INVALID: L2 入口只能执行 L2-* 场景".into());
    }
    let run_id = new_run_id("l2");
    let (summary, metrics) = execute_l2(&config, &run_id).await?;
    EvidenceWriter::create(root, &run_id)?.write(&config, &summary, &metrics)?;
    println!(
        "PERF_RUN_COMPLETE run_id={} verdict={:?} evidence=perf/reports/{}",
        run_id, summary.verdict, run_id
    );
    if matches!(summary.verdict, crate::domain::Verdict::Fail) {
        Err("PERF_SLO_VIOLATION: L2 verdict FAIL".into())
    } else {
        Ok(())
    }
}
