mod run;

#[cfg(feature = "sdk-metrics")]
pub use run::run_faults;
pub use run::{run_l1, run_l2, validate_contracts};
