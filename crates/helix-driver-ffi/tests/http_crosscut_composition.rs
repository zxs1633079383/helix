use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;
use std::time::Duration;

use helix_driver_ffi::api::{
    helix_command, helix_create_with_network, helix_destroy, helix_remove_header, helix_set_header,
    FFI_OK,
};
use helix_driver_ffi::vtable::CppHostVtable;

extern "C" fn noop_events(_ctx: *mut std::ffi::c_void, _ptr: *const u8, _len: usize) {}

extern "C" fn fixed_now_ms(_ctx: *mut std::ffi::c_void) -> u64 {
    1_700_000_000_000
}

fn test_vtable() -> CppHostVtable {
    CppHostVtable {
        on_domain_events_batch: Some(noop_events),
        now_ms: Some(fixed_now_ms),
        ctx: std::ptr::null_mut(),
    }
}

fn spawn_header_capture_server() -> (String, mpsc::Receiver<Vec<(String, String)>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind local HTTP capture server");
    let addr = listener.local_addr().expect("local addr");
    let (tx, rx) = mpsc::channel();

    std::thread::spawn(move || {
        let Ok((mut stream, _peer)) = listener.accept() else {
            return;
        };
        let _ = stream.set_read_timeout(Some(Duration::from_secs(3)));

        let mut buf = Vec::new();
        let mut chunk = [0_u8; 1024];
        while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
            match stream.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => buf.extend_from_slice(&chunk[..n]),
                Err(_) => break,
            }
        }

        let text = String::from_utf8_lossy(&buf);
        let headers = text
            .split("\r\n")
            .skip(1)
            .take_while(|line| !line.is_empty())
            .filter_map(|line| {
                let (name, value) = line.split_once(':')?;
                Some((name.trim().to_ascii_lowercase(), value.trim().to_string()))
            })
            .collect::<Vec<_>>();
        let _ = tx.send(headers);

        let _ = stream.write_all(
            b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\ncontent-type: application/json\r\n\r\n{}",
        );
    });

    (format!("http://{addr}"), rx)
}

fn header_value<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    let name = name.to_ascii_lowercase();
    headers
        .iter()
        .find(|(k, _)| k == &name)
        .map(|(_, v)| v.as_str())
}

fn create_handle(api_base: &str) -> *mut helix_driver_ffi::api::HelixHandle {
    let ws_url = b"ws://127.0.0.1:1/api/v4/websocket";
    let handle = unsafe {
        helix_create_with_network(
            api_base.as_ptr(),
            api_base.len(),
            ws_url.as_ptr(),
            ws_url.len(),
            test_vtable(),
        )
    };
    assert!(!handle.is_null(), "FFI handle should create");
    handle
}

fn set_header(handle: *mut helix_driver_ffi::api::HelixHandle, name: &str, value: &str) {
    let set_status = unsafe {
        helix_set_header(
            handle,
            name.as_ptr(),
            name.len(),
            value.as_ptr(),
            value.len(),
        )
    };
    assert_eq!(set_status, FFI_OK);
}

fn remove_header(handle: *mut helix_driver_ffi::api::HelixHandle, name: &str) {
    let remove_status = unsafe { helix_remove_header(handle, name.as_ptr(), name.len()) };
    assert_eq!(remove_status, FFI_OK);
}

fn issue_command(handle: *mut helix_driver_ffi::api::HelixHandle, command: &[u8], payload: &[u8]) {
    let command_status = unsafe {
        helix_command(
            handle,
            command.as_ptr(),
            command.len(),
            payload.as_ptr(),
            payload.len(),
        )
    };
    assert_eq!(command_status, FFI_OK);
}

fn issue_session_command(handle: *mut helix_driver_ffi::api::HelixHandle) {
    issue_command(
        handle,
        b"im_team_quit",
        br#"{"user_id":"u-crosscut","team_id":"t-crosscut"}"#,
    );
}

fn issue_bot_command(handle: *mut helix_driver_ffi::api::HelixHandle) {
    issue_command(handle, b"im_bot_agent_info", br#"{}"#);
}

#[test]
fn ffi_outbound_session_intent_injects_cookie_and_strips_internal_header() {
    let (api_base, captured) = spawn_header_capture_server();
    let handle = create_handle(&api_base);

    set_header(handle, "cookieId", "user-session-1");
    issue_session_command(handle);

    let headers = captured
        .recv_timeout(Duration::from_secs(5))
        .expect("FFI command should issue one HTTP request");
    unsafe { helix_destroy(handle) };

    assert_eq!(header_value(&headers, "cookieId"), Some("user-session-1"));
    assert!(
        header_value(&headers, "X-Auth-Kind").is_none(),
        "internal X-Auth-Kind must be stripped before FFI HTTP reaches network; headers={headers:?}"
    );
}

#[test]
fn ffi_remove_session_header_clears_session_auth_registry() {
    let (api_base, captured) = spawn_header_capture_server();
    let handle = create_handle(&api_base);

    set_header(handle, "cookieId", "user-session-1");
    remove_header(handle, "cookieId");
    issue_session_command(handle);

    let headers = captured
        .recv_timeout(Duration::from_secs(5))
        .expect("FFI command should issue one HTTP request");
    unsafe { helix_destroy(handle) };

    assert!(header_value(&headers, "cookieId").is_none());
    assert!(header_value(&headers, "X-Auth-Kind").is_none());
}

#[test]
fn ffi_outbound_bot_intent_injects_authorization_and_strips_internal_header() {
    let (api_base, captured) = spawn_header_capture_server();
    let handle = create_handle(&api_base);

    set_header(handle, "Authorization", "Bearer bot-secret");
    issue_bot_command(handle);

    let headers = captured
        .recv_timeout(Duration::from_secs(5))
        .expect("FFI command should issue one HTTP request");
    unsafe { helix_destroy(handle) };

    assert_eq!(
        header_value(&headers, "Authorization"),
        Some("Bearer bot-secret")
    );
    assert!(header_value(&headers, "X-Auth-Kind").is_none());
}

#[test]
fn ffi_remove_bot_header_clears_bot_auth_registry() {
    let (api_base, captured) = spawn_header_capture_server();
    let handle = create_handle(&api_base);

    set_header(handle, "Authorization", "Bearer bot-secret");
    remove_header(handle, "Authorization");
    issue_bot_command(handle);

    let headers = captured
        .recv_timeout(Duration::from_secs(5))
        .expect("FFI command should issue one HTTP request");
    unsafe { helix_destroy(handle) };

    assert!(header_value(&headers, "Authorization").is_none());
    assert!(header_value(&headers, "X-Auth-Kind").is_none());
}
