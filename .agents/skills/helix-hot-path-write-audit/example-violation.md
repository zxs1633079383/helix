# Example: cses-client push_msg (2026-05-17 incident)

This is the real case that motivated the **Hot-Path Write Complexity** rule.
Code lived in `cses-client/src-tauri/src/.../message/push.rs`.

---

## The violation (before)

```rust
// hot path: triggered once per inbound WS push (1k+ events/sec peak)
pub async fn push_msg(
    db: &SurrealDb,
    payload: PushPayload,
) -> Result<(), Error> {
    // VIOLATION 1: payload.props.clone() — first deep copy of the whole JSON blob
    let props_copy = payload.props.clone();

    // VIOLATION 2: serde_json::to_string(&wire_message) — second blob round-trip
    let wire_message = WireMessage {
        id:         payload.id.clone(),       // VIOLATION 3: a 2nd .clone() in same fn
        channel_id: payload.channel_id.clone(),
        seq:        payload.seq,
        sender_id:  payload.sender_id.clone(),
        content:    payload.content.clone(),  // VIOLATION 4: 3rd .clone()
        props:      props_copy,
        created_at: payload.created_at,
    };
    let wire_json = serde_json::to_string(&wire_message)?;  // <-- wire-json-roundtrip

    // VIOLATION 5: writes a single TEXT column containing the whole wire JSON
    db.query(
        "INSERT INTO messages (id, channel_id, props) VALUES ($id, $cid, $props)"
    )
    .bind(("id", payload.id))
    .bind(("cid", payload.channel_id))
    .bind(("props", wire_json))   // <-- entire wire object as one TEXT column
    .await?;
    Ok(())
}
```

Cost per event:
- 2 × `String::clone` on `props` + `content` (each can be 1–4 KB)
- 1 × `serde_json::to_string` of the whole `WireMessage`
- 1 × `INSERT` writing a denormalized blob

Peak throughput (1k events/sec) → ~8–16 MB/s of pure GC pressure, plus
permanent schema debt: `props` becomes unindexable, every future protocol
field gets buried in the blob.

---

## audit.sh output

```
$ bash ~/.claude/skills/hot-path-write-audit/audit.sh push.rs
[hot-path-violation] push.rs:18: wire-json-roundtrip
  matched: let wire_json = serde_json::to_string(&wire_message)?;
  rule:    禁止 serde_json::to_string(整条 wire 对象) 塞单列
  fix:     把协议级稳定字段拆独立列；只有业务自由扩展才进 props (且独立列)。
[hot-path-violation] push.rs:9: multi-clone-per-fn
  matched: push_msg:9   let props_copy = payload.props.clone();
  matched: push_msg:12  id:         payload.id.clone(),
  matched: push_msg:13  channel_id: payload.channel_id.clone(),
  ...
  rule:    一个函数体出现 ≥ 2 次 .clone() (实际 7 次) — 热路径深拷贝堆叠
  fix:     用借用 / Arc / 字段拆分；只在出参 boundary 处必要时 clone 一次

2 violation(s) in push.rs
```

---

## The fix (after)

### Step 1 — schema: pull stable protocol fields into their own columns

```sql
-- Before
CREATE TABLE messages (
    id         TEXT PRIMARY KEY,
    channel_id TEXT NOT NULL,
    props      TEXT NOT NULL DEFAULT '{}'   -- whole wire blob
);

-- After: protocol-stable fields are first-class columns
CREATE TABLE messages (
    id         TEXT PRIMARY KEY,
    channel_id TEXT NOT NULL,
    sender_id  TEXT NOT NULL,
    seq        INTEGER NOT NULL,
    msg_type   INTEGER NOT NULL,
    is_urgent  BOOLEAN NOT NULL DEFAULT 0,
    content    TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    props      TEXT NOT NULL DEFAULT '{}'   -- ONLY business-side free-form extension goes here
);
CREATE INDEX idx_messages_channel_seq ON messages(channel_id, seq);
```

### Step 2 — handler: column-bind, no whole-object marshal, single move

```rust
pub async fn push_msg(
    db: &SurrealDb,
    payload: PushPayload,    // move, not borrow — caller no longer needs it
) -> Result<(), Error> {
    // No .clone(). No serde_json::to_string(&wire_message).
    // SurrealDB driver binds each column directly; props is bound as-is
    // (it's already a String — we never reconstruct WireMessage).
    db.query(
        "INSERT INTO messages
         (id, channel_id, sender_id, seq, msg_type, is_urgent, content, created_at, props)
         VALUES ($id, $cid, $sid, $seq, $mt, $urg, $content, $ts, $props)"
    )
    .bind(("id",      payload.id))
    .bind(("cid",     payload.channel_id))
    .bind(("sid",     payload.sender_id))
    .bind(("seq",     payload.seq))
    .bind(("mt",      payload.msg_type))
    .bind(("urg",     payload.is_urgent))
    .bind(("content", payload.content))
    .bind(("ts",      payload.created_at))
    .bind(("props",   payload.props))   // bind the business extension as-is
    .await?;
    Ok(())
}
```

Cost per event after:
- 0 × `String::clone`
- 0 × full-object JSON encode
- 1 × `INSERT` with 9 typed column binds (O(1))

---

## Mention queue (the same rule, normalized form)

The push handler also accumulates per-recipient mention state. The
**wrong** way is `UPDATE channel_member SET mentions = ?` where `mentions`
is a JSON array column (RMW = O(N) per push). The **right** way:

```sql
CREATE TABLE channel_member_mention (
    channel_id TEXT    NOT NULL,
    user_id    TEXT    NOT NULL,
    msg_id     TEXT    NOT NULL,
    seq        INTEGER NOT NULL,
    sender_id  TEXT    NOT NULL,
    PRIMARY KEY (channel_id, user_id, msg_id)
);
CREATE INDEX idx_mention_channel_user_seq
    ON channel_member_mention(channel_id, user_id, seq);
```

- Append on push: 1 × `INSERT` (O(log N) ≈ O(1)).
- Mark-read up to seq X: 1 × `DELETE WHERE channel_id=? AND user_id=? AND seq<=?`.
- Count unread: `SELECT count(*) WHERE channel_id=? AND user_id=?`, indexed.

No JSON. No RMW. No clone storm.

---

## Lessons

1. **Schema sets the ceiling.** Once `props TEXT` becomes the "bag of fields"
   habit, every future protocol field is unindexable. Pay the column-design
   cost upfront.
2. **`.clone()` in a hot path is a smell, not a feature.** If you find
   yourself cloning to construct an intermediate object that gets serialized
   immediately, you can probably bind columns directly from the source.
3. **Normalized > arrays for accumulation.** A JSON array column for any
   queue/list is O(N) per write at minimum, O(N²) per channel over time.
   Always use a child table with composite PK.
4. **Audit at write time, not at incident time.** This skill exists so the
   next push handler trips the lint *before* it ships, not after pprof.
