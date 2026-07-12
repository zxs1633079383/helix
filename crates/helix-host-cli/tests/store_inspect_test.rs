use helix_host_cli::store_inspect::{inspect, InspectRequest};
use rusqlite::Connection;

fn fixture_request(account: &str) -> InspectRequest {
    let db = std::env::temp_dir().join(format!("helix-{account}-store-inspect.sqlite"));
    let connection = Connection::open(&db).unwrap();
    connection.execute_batch("CREATE TABLE IF NOT EXISTS message (id TEXT, channel_id TEXT, body TEXT); DELETE FROM message; INSERT INTO message VALUES ('post-1', 'channel-1', 'secret message body');").unwrap();
    InspectRequest::parse(["--db", db.to_str().unwrap(), "--account-id", account, "--table", "message", "--id", "post-1"]).unwrap()
}

#[test]
fn rejects_unknown_table_and_unbounded_scan() {
    assert!(InspectRequest::parse(["--db", "/tmp/444.db", "--account-id", "444", "--table", "sqlite_master"]).is_err());
    assert!(InspectRequest::parse(["--db", "/tmp/444.db", "--account-id", "444", "--table", "message", "--limit", "10000"]).is_err());
}

#[test]
fn reads_only_requested_account_store_and_redacts_message() {
    let result = inspect(&fixture_request("444")).unwrap();
    assert_eq!(result.account_id, "444");
    assert!(!result.json.contains("secret message body"));
    assert!(result.json.contains("post-1"));
}
