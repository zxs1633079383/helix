mod application;
mod domain;
mod infrastructure;

use std::path::{Path, PathBuf};

#[cfg(feature = "sdk-metrics")]
use application::run_faults;
use application::{run_l1, run_l2, validate_contracts};
use domain::ScenarioConfig;

#[global_allocator]
static PERF_ALLOCATOR: infrastructure::process_profile::CountingAllocator =
    infrastructure::process_profile::CountingAllocator;

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    if let Err(error) = dispatch().await {
        eprintln!("{error}");
        std::process::exit(2);
    }
}

async fn dispatch() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = args.first().map(String::as_str).unwrap_or("help");
    let root = workspace_root()?;
    match command {
        "preflight" => preflight(&root),
        "validate-contracts" => validate_contracts(&root),
        "l1" => run_l1(&root, load_config(&root, "l1.toml")?).await,
        "l2" => run_l2(&root, load_config(&root, "l2.toml")?).await,
        #[cfg(feature = "sdk-metrics")]
        "faults" => run_faults(&root),
        "run" => {
            let scenario = value_after(&args, "--scenario")
                .ok_or("PERF_CONFIG_INVALID: run 缺少 --scenario")?;
            if scenario == "L1-STEP-INORDER" {
                let mut config = load_config(&root, "l1.toml")?;
                config.run.scenario = scenario.to_string();
                run_l1(&root, config).await
            } else if scenario == "L2-ENGINE-MIXED" {
                let mut config = load_config(&root, "l2.toml")?;
                config.run.scenario = scenario.to_string();
                run_l2(&root, config).await
            } else {
                Err(format!(
                    "PERF_PROVIDER_MISSING: {scenario} 尚无真实 provider，拒绝伪造 PASS"
                ))
            }
        }
        "help" | "--help" | "-h" => {
            println!("helix-perf <preflight|validate-contracts|l1|l2|faults|run --scenario ID>");
            Ok(())
        }
        _ => Err(format!("PERF_CONFIG_INVALID: 未知命令 {command}")),
    }
}

fn preflight(root: &Path) -> Result<(), String> {
    validate_contracts(root)?;
    for file in ["default.toml", "l1.toml", "l2.toml"] {
        load_config(root, file)?;
    }
    println!("PERF_PREFLIGHT_OK: 本地 L1/L2 合同与场景有效");
    Ok(())
}

fn load_config(root: &Path, file: &str) -> Result<ScenarioConfig, String> {
    ScenarioConfig::load(&root.join("perf/scenarios").join(file))
}

fn workspace_root() -> Result<PathBuf, String> {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .ok_or_else(|| "PERF_CONFIG_INVALID: 无法定位 workspace root".to_string())
}

fn value_after<'a>(args: &'a [String], key: &str) -> Option<&'a str> {
    args.iter()
        .position(|arg| arg == key)
        .and_then(|index| args.get(index + 1))
        .map(String::as_str)
}
