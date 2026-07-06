use std::fs;
use std::path::PathBuf;

#[test]
fn generated_headers_export_command_with_trace() {
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let header_c = fs::read_to_string(crate_dir.join("helix.h")).expect("read helix.h");
    let header_cpp = fs::read_to_string(crate_dir.join("helix.hpp")).expect("read helix.hpp");

    assert!(
        header_c.contains("helix_command_with_trace"),
        "helix.h should export helix_command_with_trace"
    );
    assert!(
        header_cpp.contains("helix_command_with_trace"),
        "helix.hpp should export helix_command_with_trace"
    );
}

#[test]
fn trace_command_source_starts_mobile_ffi_boundary_span() {
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let source =
        fs::read_to_string(crate_dir.join("src/api.rs")).expect("read helix-driver-ffi src/api.rs");

    assert!(
        source.contains("\"mobile.ffi.command\""),
        "helix_command_with_trace should start the mobile.ffi.command span"
    );
    assert!(
        source.contains("TraceDirection::Inbound"),
        "mobile FFI trace boundary should be marked inbound"
    );
    assert!(
        source.contains("HostOtelRuntime::from_env(\"helix-driver-ffi\")"),
        "FFI handle should load an env-configurable host OTel runtime"
    );
}
