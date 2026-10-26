//! Real SQLite regression for the app-owned message identity cleanup.
use rusqlite::Connection;
use serde_json::{json, Value};

/// Startup migrations preserve content, IDs, and business profiles while removing old names.
#[test]
fn legacy_message_profiles_are_removed_without_losing_content_or_directory() {
    let db = Connection::open_in_memory().unwrap();
    for sql in helix_im::IM_SCHEMA
        .iter()
        .chain(helix_im::IM_SCHEMA_MIGRATIONS)
    {
        if let Err(error) = db.execute_batch(sql) {
            assert!(
                error.to_string().contains("duplicate column name"),
                "{sql}: {error}"
            );
        }
    }
    db.execute_batch("CREATE TABLE directory_fixture (user_name TEXT); INSERT INTO directory_fixture VALUES ('雷霆')").unwrap();
    let literal_body = r#"{"userSnapshot":{"userName":"正文不是身份"}}"#;
    let nested = json!([{ "userSnapshot":{"userId":"nested-author","teamId":"nested-company","userName":"旧名"}, "message":literal_body }]);
    let archived = json!({"messages":[{ "userSnapshot":{"userId":"archived-author","teamId":"archived-company","userName":"旧名"}, "message":"原始归档正文", "props":{"mergeMessage":nested.to_string()} }]});
    let props = json!({"mergeMessage":archived.to_string(),"merge_message":"invalid-json-preserved","task":{"name":"业务正文"},"forward":{"snapshotId":"s1","preview":[{"author":"旧名","userId":"u2","teamId":"c2","text":"内容归档"}]}});
    let replies = json!([{"replyUserName":"旧名","replyUserId":"u2","userSnapshot":{"userId":"u2","teamId":"c2","userName":"旧名"},"message":"回复正文"}]);
    db.execute("INSERT INTO message (temporary_id,user_snapshot,props,replied_message,reply_messages) VALUES ('t1',?1,?2,?3,?4)",
        rusqlite::params![json!({"userId":"u1","teamId":"c1","userName":"旧名"}).to_string(),props.to_string(),json!({"repliedUserId":"u2","repliedUserName":"旧名","message":"引用正文"}).to_string(),replies.to_string()]).unwrap();
    for _ in 0..2 {
        for sql in helix_im::IM_SCHEMA_MIGRATIONS {
            if let Err(error) = db.execute_batch(sql) {
                assert!(
                    error.to_string().contains("duplicate column name"),
                    "{sql}: {error}"
                );
            }
        }
    }
    let (user,team,snapshot,props,reply,replies): (String,String,String,String,String,String) = db.query_row(
        "SELECT user_id,team_id,user_snapshot,props,replied_message,reply_messages FROM message WHERE temporary_id='t1'",[],
        |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).unwrap();
    assert_eq!(
        (user.as_str(), team.as_str(), snapshot.as_str()),
        ("u1", "c1", "")
    );
    let props: Value = serde_json::from_str(&props).unwrap();
    let reply: Value = serde_json::from_str(&reply).unwrap();
    let replies: Value = serde_json::from_str(&replies).unwrap();
    assert_eq!(props["task"]["name"], "业务正文");
    let archived: Value = serde_json::from_str(props["mergeMessage"].as_str().unwrap()).unwrap();
    let nested: Value = serde_json::from_str(
        archived["messages"][0]["props"]["mergeMessage"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(archived["messages"][0]["message"], "原始归档正文");
    assert_eq!(archived["messages"][0]["userId"], "archived-author");
    assert_eq!(archived["messages"][0]["teamId"], "archived-company");
    assert!(archived["messages"][0].get("userSnapshot").is_none());
    assert_eq!(nested[0]["userId"], "nested-author");
    assert_eq!(nested[0]["teamId"], "nested-company");
    assert_eq!(nested[0]["message"], literal_body);
    assert!(nested[0].get("userSnapshot").is_none());
    assert_eq!(props["merge_message"], "invalid-json-preserved");
    assert_eq!(props["forward"]["preview"][0]["text"], "内容归档");
    assert!(props["forward"]["preview"][0].get("author").is_none());
    assert_eq!(reply["message"], "引用正文");
    assert!(reply.get("repliedUserName").is_none());
    assert_eq!(replies[0]["userId"], "u2");
    assert_eq!(replies[0]["teamId"], "c2");
    assert_eq!(replies[0]["message"], "回复正文");
    assert!(replies[0].get("userSnapshot").is_none());
    assert!(replies[0].get("replyUserName").is_none());
    let name: String = db
        .query_row("SELECT user_name FROM directory_fixture", [], |r| r.get(0))
        .unwrap();
    assert_eq!(name, "雷霆");
}
