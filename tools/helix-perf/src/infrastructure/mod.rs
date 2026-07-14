mod evidence;
mod ffi_probe;
mod hdr;
mod l1_driver;
mod l1_metrics;
mod l1_oracle;
mod l2_case;
mod l2_driver;
mod l2_fixture;
mod l2_metrics;
mod l2_module;
mod l2_oracle;
mod metric_sink;

pub use evidence::{new_run_id, validate_json_contracts, EvidenceWriter};
pub use l1_driver::execute_l1;
pub use l2_driver::execute_l2;
pub use metric_sink::ProbeSnapshot;
