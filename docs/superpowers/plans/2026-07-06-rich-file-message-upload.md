# Rich/File Message Upload Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add rich image and single-file message sending so Helix stores the local message first, uploads local files through presigned URLs, updates only `props` upload state per file, and calls `posts/create` only after all files succeed.

**Architecture:** `helix-im` owns message rules, pending send state, `props` mutation, and the decision to call `posts/create`. `helix-core` adds only a runtime-neutral `UploadFile` effect and `FileUploader` port. `helix-driver-host` performs real local-file streaming upload; native and FFI stay thin over host driver logic.

**Tech Stack:** Rust, `serde_json`, existing `StorageOp::BatchUpsert`, existing `BoundedSpawner`, `reqwest`, `tokio`, existing `PortReply`/`Correlation` routing.

## Global Constraints

- Do not add `attachments` as a post top-level field.
- Do not add or rename `SendStatus` variants; keep `Local`, `Sending`, `Sent`, `UnSend`.
- `message.send_status` remains Post send status only.
- File upload status lives only in `props.images[i].upload.status` or `props.file.upload.status`.
- Helix recognizes only upload routing fields: `props.textPosition`, `props.images[]`, `props.file`, `localPath`, `upload.status`, `upload.method`, `upload.url`, `upload.headers`, `upload.objectKey`.
- Helix preserves all other `props` fields without interpreting them, including image dimensions, thumbnail data, preview data, and layout data.
- Driver must not parse IM props and must not write the database.
- `helix-core` must not import OSS, Alibaba Cloud, Tauri, reqwest, tokio/fs, sqlx, or platform APIs.
- First version does not implement in-flight upload cancellation, multipart upload, resumable upload, or dedup/instant upload.
- Cargo discipline: no cargo during mid-edit loops; run focused tests after each task and full checks only at task/phase boundaries.

---

## File Structure

- Create `crates/helix-im/src/send/upload_props.rs`: pure parser/validator/mutator for rich/file upload props.
- Modify `crates/helix-im/src/send/mod.rs`: export `upload_props`.
- Create `crates/helix-im/tests/rich_file_upload_contract_test.rs`: command-level contract tests for rich/file validation and upload gating.
- Modify `crates/helix-core/src/effect.rs`: add `FileUploadRequest`, `FileUploadResponse`, and `Effect::UploadFile`.
- Create `crates/helix-core/src/ports/upload.rs`: add `FileUploader` trait.
- Modify `crates/helix-core/src/ports/mod.rs`: export `FileUploader`.
- Modify `crates/helix-driver-host/src/owned_effect.rs`: clone/own `UploadFile` effects.
- Modify `crates/helix-driver-host/src/pools.rs`: add upload worker pool helper.
- Modify `crates/helix-driver-host/src/dispatch.rs`: dispatch `OwnedEffect::UploadFile`.
- Modify `crates/helix-driver-host/src/engine.rs`: add `FileUploader` dependency and upload pool wiring.
- Create `crates/helix-driver-host/src/upload.rs`: reqwest-backed local path uploader.
- Modify `crates/helix-driver-host/src/lib.rs`: export upload types.
- Modify `crates/helix-driver-native/src/engine_loop.rs` and FFI pump wiring as needed to pass host uploader into shared engine.
- Modify `crates/helix-im/src/state.rs`: add `pending_uploads`.
- Modify `crates/helix-im/src/send/pending.rs`: store pending body and remaining uploads.
- Modify `crates/helix-im/src/send/flow.rs`: branch rich/file with uploads before `posts/create`.
- Modify `crates/helix-im/src/port_reply.rs`: handle upload replies.
- Create `crates/helix-im/src/send/retry_upload.rs`: retry failed file upload by `temporary_id` and target.
- Modify `crates/helix-im/src/module.rs`: route `im_retry_upload`.
- Add focused tests under `crates/helix-driver-host/src/upload.rs` or `crates/helix-driver-host/tests/file_upload.rs` depending on test ergonomics.

---

### Task 1: Rich/File Props Parser And Validation

**Files:**
- Create: `crates/helix-im/src/send/upload_props.rs`
- Modify: `crates/helix-im/src/send/mod.rs`
- Test: `crates/helix-im/tests/rich_file_upload_contract_test.rs`

**Interfaces:**
- Consumes: `serde_json::Value`, existing `ImError`.
- Produces:
  - `UploadPlan { props: Value, targets: Vec<UploadTarget> }`
  - `UploadTarget::RichImage { index: usize }`
  - `UploadTarget::File`
  - `build_upload_plan(msg_type: &str, message: &str, props: Value) -> Result<UploadPlan, ImError>`
  - `mark_upload_success(props: &mut Value, target: &UploadTarget, result: &UploadSuccess)`
  - `mark_upload_failed(props: &mut Value, target: &UploadTarget, error: &str)`

- [ ] **Step 1: Write parser unit tests**

Add `#[cfg(test)]` tests in `crates/helix-im/src/send/upload_props.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn rich_accepts_images_and_preserves_frontend_fields() {
        let props = json!({
            "textPosition": "bottom",
            "images": [{
                "localPath": "/tmp/a.png",
                "name": "a.png",
                "width": 800,
                "upload": {
                    "status": "pending",
                    "method": "PUT",
                    "url": "https://upload",
                    "headers": {"x-oss": "1"},
                    "objectKey": "im/a.png"
                }
            }],
            "frontendCustom": {"keep": true}
        });

        let plan = build_upload_plan("rich", "hello", props).expect("rich image plan");

        assert_eq!(plan.targets, vec![UploadTarget::RichImage { index: 0 }]);
        assert_eq!(plan.props["images"][0]["upload"]["status"], "uploading");
        assert_eq!(plan.props["images"][0]["width"], 800);
        assert_eq!(plan.props["frontendCustom"]["keep"], true);
    }

    #[test]
    fn rich_rejects_file_field() {
        let err = build_upload_plan(
            "rich",
            "hello",
            json!({"textPosition":"top","images":[],"file":{"localPath":"/tmp/a.pdf"}}),
        )
        .unwrap_err();

        assert!(err.to_string().contains("rich"));
    }

    #[test]
    fn file_rejects_message_text() {
        let props = json!({
            "file": {
                "localPath": "/tmp/a.pdf",
                "upload": {"method":"PUT","url":"https://upload","headers":{},"objectKey":"im/a.pdf"}
            }
        });

        let err = build_upload_plan("file", "not allowed", props).unwrap_err();

        assert!(err.to_string().contains("file"));
    }

    #[test]
    fn mark_success_updates_only_target_upload_node() {
        let mut props = json!({
            "textPosition":"bottom",
            "images":[
                {"localPath":"/tmp/a.png","upload":{"status":"uploading","objectKey":"im/a.png"}},
                {"localPath":"/tmp/b.png","upload":{"status":"uploading","objectKey":"im/b.png"}}
            ],
            "frontendCustom": 7
        });

        mark_upload_success(
            &mut props,
            &UploadTarget::RichImage { index: 1 },
            &UploadSuccess {
                object_key: "im/b.png".to_string(),
                url: Some("https://cdn/b.png".to_string()),
                etag: Some("etag-b".to_string()),
            },
        )
        .expect("success mutation");

        assert_eq!(props["images"][0]["upload"]["status"], "uploading");
        assert_eq!(props["images"][1]["upload"]["status"], "success");
        assert_eq!(props["images"][1]["upload"]["etag"], "etag-b");
        assert_eq!(props["frontendCustom"], 7);
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run:

```bash
cargo test -p helix-im rich_file_upload_contract -- --nocapture
```

Expected: compile failure because `upload_props` and its functions are not defined.

- [ ] **Step 3: Implement minimal parser/mutator**

Create `crates/helix-im/src/send/upload_props.rs`:

```rust
use crate::ImError;
use serde_json::{json, Value};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum UploadTarget {
    RichImage { index: usize },
    File,
}

#[derive(Debug, Clone)]
pub struct UploadPlan {
    pub props: Value,
    pub targets: Vec<UploadTarget>,
}

#[derive(Debug, Clone)]
pub struct UploadSuccess {
    pub object_key: String,
    pub url: Option<String>,
    pub etag: Option<String>,
}

pub fn build_upload_plan(
    msg_type: &str,
    message: &str,
    mut props: Value,
) -> Result<UploadPlan, ImError> {
    match msg_type {
        "rich" => build_rich_plan(&mut props),
        "file" => build_file_plan(message, &mut props),
        _ => Ok(UploadPlan { props, targets: Vec::new() }),
    }
}

fn build_rich_plan(props: &mut Value) -> Result<UploadPlan, ImError> {
    if props.get("file").is_some() {
        return Err(ImError::Parse("rich message must not contain props.file".to_string()));
    }
    match props.get("textPosition").and_then(Value::as_str) {
        Some("top" | "bottom") => {}
        _ => return Err(ImError::Parse("rich message requires textPosition top/bottom".to_string())),
    }
    let images = props
        .get_mut("images")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| ImError::Parse("rich message requires props.images".to_string()))?;
    if images.is_empty() {
        return Err(ImError::Parse("rich message requires at least one image".to_string()));
    }

    let mut targets = Vec::with_capacity(images.len());
    for (index, image) in images.iter_mut().enumerate() {
        validate_upload_node(image, "rich image")?;
        set_status(image, "uploading")?;
        targets.push(UploadTarget::RichImage { index });
    }
    Ok(UploadPlan { props: props.clone(), targets })
}

fn build_file_plan(message: &str, props: &mut Value) -> Result<UploadPlan, ImError> {
    if !message.trim().is_empty() {
        return Err(ImError::Parse("file message must not contain message text".to_string()));
    }
    if props.get("images").is_some() {
        return Err(ImError::Parse("file message must not contain props.images".to_string()));
    }
    let file = props
        .get_mut("file")
        .ok_or_else(|| ImError::Parse("file message requires props.file".to_string()))?;
    validate_upload_node(file, "file")?;
    set_status(file, "uploading")?;
    Ok(UploadPlan { props: props.clone(), targets: vec![UploadTarget::File] })
}

fn validate_upload_node(node: &Value, label: &str) -> Result<(), ImError> {
    if node.get("localPath").and_then(Value::as_str).unwrap_or("").is_empty() {
        return Err(ImError::Parse(format!("{label} requires localPath")));
    }
    let upload = node
        .get("upload")
        .ok_or_else(|| ImError::Parse(format!("{label} requires upload")))?;
    for key in ["method", "url", "objectKey"] {
        if upload.get(key).and_then(Value::as_str).unwrap_or("").is_empty() {
            return Err(ImError::Parse(format!("{label} upload requires {key}")));
        }
    }
    if !upload.get("headers").map(Value::is_object).unwrap_or(false) {
        return Err(ImError::Parse(format!("{label} upload.headers must be object")));
    }
    Ok(())
}

fn set_status(node: &mut Value, status: &str) -> Result<(), ImError> {
    node.get_mut("upload")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| ImError::Parse("upload must be object".to_string()))?
        .insert("status".to_string(), json!(status));
    Ok(())
}

pub fn mark_upload_success(
    props: &mut Value,
    target: &UploadTarget,
    result: &UploadSuccess,
) -> Result<(), ImError> {
    let node = target_node_mut(props, target)?;
    let upload = node
        .get_mut("upload")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| ImError::Parse("upload must be object".to_string()))?;
    upload.insert("status".to_string(), json!("success"));
    upload.insert("objectKey".to_string(), json!(result.object_key));
    if let Some(url) = &result.url {
        upload.insert("url".to_string(), json!(url));
    }
    if let Some(etag) = &result.etag {
        upload.insert("etag".to_string(), json!(etag));
    }
    Ok(())
}

pub fn mark_upload_failed(
    props: &mut Value,
    target: &UploadTarget,
    error: &str,
) -> Result<(), ImError> {
    let node = target_node_mut(props, target)?;
    let upload = node
        .get_mut("upload")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| ImError::Parse("upload must be object".to_string()))?;
    upload.insert("status".to_string(), json!("failed"));
    upload.insert("error".to_string(), json!(error));
    Ok(())
}

fn target_node_mut<'a>(props: &'a mut Value, target: &UploadTarget) -> Result<&'a mut Value, ImError> {
    match target {
        UploadTarget::RichImage { index } => props
            .get_mut("images")
            .and_then(Value::as_array_mut)
            .and_then(|images| images.get_mut(*index))
            .ok_or_else(|| ImError::Parse("missing rich image target".to_string())),
        UploadTarget::File => props
            .get_mut("file")
            .ok_or_else(|| ImError::Parse("missing file target".to_string())),
    }
}
```

Modify `crates/helix-im/src/send/mod.rs`:

```rust
pub(crate) mod flow;
pub mod pending;
pub mod pong_compensate;
pub mod reconcile;
pub mod upload_props;
```

- [ ] **Step 4: Run focused tests**

Run:

```bash
cargo test -p helix-im upload_props -- --nocapture
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/helix-im/src/send/mod.rs crates/helix-im/src/send/upload_props.rs
git commit -m "feat(im): 校验 rich/file 上传 props"
```

---

### Task 2: Runtime-Neutral UploadFile Effect And Host Driver Upload Port

**Files:**
- Modify: `crates/helix-core/src/effect.rs`
- Create: `crates/helix-core/src/ports/upload.rs`
- Modify: `crates/helix-core/src/ports/mod.rs`
- Modify: `crates/helix-driver-host/src/owned_effect.rs`
- Modify: `crates/helix-driver-host/src/pools.rs`
- Modify: `crates/helix-driver-host/src/dispatch.rs`
- Modify: `crates/helix-driver-host/src/engine.rs`
- Create: `crates/helix-driver-host/src/upload.rs`
- Modify: `crates/helix-driver-host/src/lib.rs`
- Modify: `crates/helix-driver-native/src/engine_loop.rs`
- Modify: `crates/helix-driver-ffi/src/pump.rs`

**Interfaces:**
- Consumes: `FileUploadRequest` emitted by modules.
- Produces:
  - `Effect::UploadFile { corr, req }`
  - `FileUploader::upload(req) -> Result<FileUploadResponse, PortError>`
  - `FileUploadResponse` encoded into existing `PortReply` response bytes.

- [ ] **Step 1: Add core compile tests**

Add tests in `crates/helix-core/src/effect.rs`:

```rust
#[cfg(test)]
mod upload_effect_tests {
    use super::*;

    #[test]
    fn upload_file_effect_carries_path_not_bytes() {
        let req = FileUploadRequest {
            local_path: "/tmp/a.png".to_string(),
            object_key: "im/a.png".to_string(),
            method: "PUT".to_string(),
            url: "https://upload".to_string(),
            headers: vec![("x-oss".to_string(), "1".to_string())],
            content_type: Some("image/png".to_string()),
            size: Some(123),
        };

        let effect = Effect::UploadFile {
            corr: Correlation::from_raw(9),
            req,
        };

        match effect {
            Effect::UploadFile { corr, req } => {
                assert_eq!(corr.raw(), 9);
                assert_eq!(req.local_path, "/tmp/a.png");
                assert_eq!(req.object_key, "im/a.png");
                assert_eq!(req.method, "PUT");
            }
            _ => panic!("expected upload effect"),
        }
    }
}
```

Run:

```bash
cargo test -p helix-core upload_file_effect_carries_path_not_bytes
```

Expected: compile failure until core types are added.

- [ ] **Step 2: Add core types and port**

In `crates/helix-core/src/effect.rs` add near `HttpRequest`:

```rust
#[derive(Debug, Clone)]
pub struct FileUploadRequest {
    pub local_path: String,
    pub object_key: String,
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub content_type: Option<String>,
    pub size: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct FileUploadResponse {
    pub object_key: String,
    pub url: Option<String>,
    pub etag: Option<String>,
}
```

Add to `Effect`:

```rust
UploadFile {
    corr: Correlation,
    req: FileUploadRequest,
},
```

Create `crates/helix-core/src/ports/upload.rs`:

```rust
//! FileUploader port - local file upload through presigned URLs.

use crate::effect::{FileUploadRequest, FileUploadResponse};
use crate::platform::{MaybeSend, MaybeSync};
use crate::PortError;

#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
pub trait FileUploader: MaybeSend + MaybeSync + 'static {
    async fn upload(&self, req: FileUploadRequest) -> Result<FileUploadResponse, PortError>;
}
```

Modify `crates/helix-core/src/ports/mod.rs`:

```rust
pub mod upload;
pub use upload::FileUploader;
```

- [ ] **Step 3: Wire owned effects and dispatch**

Update `crates/helix-driver-host/src/owned_effect.rs`:

```rust
use helix_core::effect::{
    Correlation, DomainEventBytes, Effect, FileUploadRequest, HttpRequest, StorageOp, TimerId,
    TransportId,
};

pub enum OwnedEffect {
    // existing variants...
    UploadFile {
        corr: Correlation,
        req: FileUploadRequest,
    },
}
```

Add match arm in `consume_effects`:

```rust
Effect::UploadFile { corr, req } => OwnedEffect::UploadFile {
    corr: *corr,
    req: req.clone(),
},
```

Update dispatch signature to accept upload pool:

```rust
upload_pool: &BoundedSpawner<FileUploadRequest>,
```

Add dispatch arm:

```rust
OwnedEffect::UploadFile { corr, req } => {
    upload_pool
        .submit(Job {
            corr: Some(corr),
            payload: req,
        })
        .await;
}
```

- [ ] **Step 4: Add uploader pool helper**

In `crates/helix-driver-host/src/pools.rs` add:

```rust
use helix_core::effect::FileUploadRequest;
use helix_core::ports::FileUploader;

pub fn spawn_upload_pool<U>(
    n: usize,
    uploader: Arc<U>,
    reply_tx: mpsc::UnboundedSender<Tick>,
    overflow: Overflow,
) -> BoundedSpawner<FileUploadRequest>
where
    U: FileUploader + Send + Sync + 'static,
{
    BoundedSpawner::new(
        n,
        HTTP_QUEUE_CAP,
        overflow,
        reply_tx,
        move |req: FileUploadRequest| {
            let uploader = Arc::clone(&uploader);
            async move {
                match uploader.upload(req).await {
                    Ok(resp) => {
                        let bytes = serde_json::to_vec(&serde_json::json!({
                            "objectKey": resp.object_key,
                            "url": resp.url,
                            "etag": resp.etag
                        }))
                        .expect("upload response JSON serialization cannot fail");
                        helix_core::PortOutcome::Ok(bytes.into())
                    }
                    Err(e) => helix_core::PortOutcome::Err(e),
                }
            }
        },
    )
}
```

Implement this helper by copying the existing `spawn_http_pool` shape in the same file and changing only the payload type and worker body. Keep the public signature above so `engine.rs` and `dispatch.rs` have a stable interface.

- [ ] **Step 5: Add host uploader**

Create `crates/helix-driver-host/src/upload.rs`:

```rust
use helix_core::effect::{FileUploadRequest, FileUploadResponse};
use helix_core::ports::FileUploader;
use helix_core::PortError;
use reqwest::{Client, Method};
use tokio::fs;

#[derive(Clone)]
pub struct SharedFileUploader {
    client: Client,
}

impl Default for SharedFileUploader {
    fn default() -> Self {
        Self { client: Client::new() }
    }
}

impl SharedFileUploader {
    pub fn new(client: Client) -> Self {
        Self { client }
    }
}

#[async_trait::async_trait]
impl FileUploader for SharedFileUploader {
    async fn upload(&self, req: FileUploadRequest) -> Result<FileUploadResponse, PortError> {
        let method = Method::from_bytes(req.method.as_bytes())
            .map_err(|e| PortError::Other(format!("invalid upload method: {e}")))?;
        if method != Method::PUT {
            return Err(PortError::Other("only PUT upload is supported".to_string()));
        }

        let bytes = fs::read(&req.local_path)
            .await
            .map_err(|e| PortError::Other(format!("read upload file failed: {e}")))?;
        if let Some(expected) = req.size {
            if bytes.len() as u64 != expected {
                return Err(PortError::Other("upload file size mismatch".to_string()));
            }
        }

        let mut builder = self.client.request(method, &req.url);
        for (name, value) in req.headers {
            builder = builder.header(&name, value);
        }
        if let Some(content_type) = req.content_type {
            builder = builder.header(reqwest::header::CONTENT_TYPE, content_type);
        }

        let response = builder
            .body(bytes)
            .send()
            .await
            .map_err(|e| PortError::Http(format!("upload failed: {e}")))?;
        let status = response.status();
        if !status.is_success() {
            return Err(PortError::Http(format!("upload status {}", status.as_u16())));
        }
        let etag = response
            .headers()
            .get(reqwest::header::ETAG)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);

        Ok(FileUploadResponse {
            object_key: req.object_key,
            url: Some(req.url),
            etag,
        })
    }
}
```

- [ ] **Step 6: Run focused checks**

Run:

```bash
cargo test -p helix-core upload_file_effect_carries_path_not_bytes
cargo check -p helix-driver-host
```

Expected: PASS after native/ffi engine wiring is updated.

- [ ] **Step 7: Commit**

```bash
git add crates/helix-core/src/effect.rs crates/helix-core/src/ports crates/helix-driver-host/src crates/helix-driver-native/src/engine_loop.rs crates/helix-driver-ffi/src/pump.rs
git commit -m "feat(core): add local file upload effect"
```

---

### Task 3: Send Flow Upload Gating Before posts/create

**Files:**
- Modify: `crates/helix-im/src/state.rs`
- Modify: `crates/helix-im/src/send/pending.rs`
- Modify: `crates/helix-im/src/send/flow.rs`
- Modify: `crates/helix-im/src/port_reply.rs`
- Test: `crates/helix-im/tests/rich_file_upload_contract_test.rs`

**Interfaces:**
- Consumes: `UploadPlan`, `Effect::UploadFile`, `FileUploadResponse` JSON.
- Produces: rich/file send path that persists local row, emits upload effects, updates props, and triggers `posts/create` only when all uploads succeed.

- [ ] **Step 1: Add command-level failing tests**

Create `crates/helix-im/tests/rich_file_upload_contract_test.rs`:

```rust
mod common;

use common::{cid, Harness};
use helix_core::Effect;
use serde_json::json;

fn upload_rich_cmd() -> helix_core::Tick {
    common::send_message_cmd_json(json!({
        "channel_id": cid(1),
        "temporary_id": "tmp-rich-1",
        "type": "rich",
        "text": "hello",
        "props": {
            "textPosition": "bottom",
            "images": [
                {"localPath":"/tmp/a.png","upload":{"method":"PUT","url":"https://upload/a","headers":{},"objectKey":"im/a.png"}},
                {"localPath":"/tmp/b.png","upload":{"method":"PUT","url":"https://upload/b","headers":{},"objectKey":"im/b.png"}}
            ],
            "frontendCustom": {"keep": true}
        }
    }))
}

#[test]
fn rich_with_two_images_persists_and_uploads_but_does_not_post_create() {
    let mut h = Harness::new(&[1]);
    let effects = h.step(upload_rich_cmd(), 1000);

    assert!(effects.iter().any(|e| matches!(e, Effect::Persist { .. })));
    assert_eq!(
        effects.iter().filter(|e| matches!(e, Effect::UploadFile { .. })).count(),
        2
    );
    assert!(
        !effects.iter().any(|e| matches!(e, Effect::Http { req, .. } if req.url.ends_with("/posts/create"))),
        "posts/create must wait for uploads"
    );
}
```

Run:

```bash
cargo test -p helix-im rich_with_two_images_persists_and_uploads_but_does_not_post_create -- --nocapture
```

Expected: compile failure or assertion failure until send flow branches on upload plan.

- [ ] **Step 2: Extend PendingSend and state**

Modify `crates/helix-im/src/send/pending.rs`:

```rust
pub struct PendingSend {
    pub temporary_id: TemporaryId,
    pub status: SendStatus,
    pub timeout_timer: TimerId,
    pub persist_corr: Option<Correlation>,
    pub connection_id: Option<String>,
    pub body: Option<serde_json::Value>,
    pub remaining_uploads: usize,
}
```

Initialize in `PendingSend::new`:

```rust
body: None,
remaining_uploads: 0,
```

Modify `crates/helix-im/src/state.rs`:

```rust
pub pending_uploads: HashMap<Correlation, crate::send::upload_props::PendingUpload>,
```

Define `PendingUpload` in `crates/helix-im/src/send/upload_props.rs`:

```rust
#[derive(Debug, Clone)]
pub struct PendingUpload {
    pub temporary_id: TemporaryId,
    pub target: UploadTarget,
}
```

- [ ] **Step 3: Emit UploadFile effects from send flow**

In `crates/helix-im/src/send/flow.rs`, after `body` is built:

```rust
let upload_plan = crate::send::upload_props::build_upload_plan(
    body["type"].as_str().unwrap_or("TEXT"),
    body["message"].as_str().unwrap_or(""),
    body.get("props").cloned().unwrap_or_else(|| serde_json::json!({})),
)?;
if !upload_plan.targets.is_empty() {
    body["props"] = upload_plan.props.clone();
}
```

After creating `PendingSend`:

```rust
ps.body = Some(body.clone());
ps.remaining_uploads = upload_plan.targets.len();
```

For no uploads, keep existing `Effect::Http { posts/create }` path.

For uploads:

```rust
for target in upload_plan.targets {
    let upload_corr = self.alloc_corr_internal();
    let req = crate::send::upload_props::file_upload_request(&body["props"], &target)?;
    self.state.pending_uploads.insert(
        upload_corr,
        crate::send::upload_props::PendingUpload {
            temporary_id: tmp_id.clone(),
            target,
        },
    );
    out.push(Effect::UploadFile { corr: upload_corr, req });
}
```

Add helper `file_upload_request` in `upload_props.rs` to convert `props + target` into `FileUploadRequest`.

- [ ] **Step 4: Handle upload PortReply**

In `crates/helix-im/src/port_reply.rs`, when `corr` is not found in existing `corr_map`, check `pending_uploads.remove(&corr)`.

On `PortOutcome::Ok(bytes)`:

```rust
let result = serde_json::from_slice::<serde_json::Value>(&bytes)
    .map_err(|e| ImError::Parse(format!("upload reply: {e}")))?;
let success = crate::send::upload_props::UploadSuccess {
    object_key: result["objectKey"].as_str().unwrap_or("").to_string(),
    url: result["url"].as_str().map(str::to_string),
    etag: result["etag"].as_str().map(str::to_string),
};
```

Then mutate `PendingSend.body.props`, persist `props`, decrement remaining, and if zero call a helper that emits existing `posts/create` HTTP effect.

On `PortOutcome::Err(e)`, mutate target upload node to `failed`, persist `props`, and do not emit `posts/create`.

- [ ] **Step 5: Run focused tests**

Run:

```bash
cargo test -p helix-im rich_file_upload_contract -- --nocapture
```

Expected: PASS for command-level upload gating tests.

- [ ] **Step 6: Commit**

```bash
git add crates/helix-im/src/state.rs crates/helix-im/src/send crates/helix-im/src/port_reply.rs crates/helix-im/tests/rich_file_upload_contract_test.rs
git commit -m "feat(im): gate rich/file send on upload success"
```

---

### Task 4: Retry Failed Upload Without Reuploading Successful Files

**Files:**
- Create: `crates/helix-im/src/send/retry_upload.rs`
- Modify: `crates/helix-im/src/send/mod.rs`
- Modify: `crates/helix-im/src/module.rs`
- Test: `crates/helix-im/tests/rich_file_upload_contract_test.rs`

**Interfaces:**
- Consumes command `im_retry_upload`.
- Produces one new `UploadFile` effect for the requested failed file target.

- [ ] **Step 1: Add failing retry test**

Append to `crates/helix-im/tests/rich_file_upload_contract_test.rs`:

```rust
#[test]
fn retry_failed_image_emits_one_upload_only() {
    let mut h = Harness::new(&[1]);
    let _ = h.step(upload_rich_cmd(), 1000);

    let retry = common::command_json("im_retry_upload", json!({
        "temporary_id": "tmp-rich-1",
        "target": {"kind": "rich_image", "index": 1}
    }));
    let effects = h.step(retry, 1100);

    assert_eq!(
        effects.iter().filter(|e| matches!(e, Effect::UploadFile { .. })).count(),
        1
    );
}
```

Add this helper to `crates/helix-im/tests/common.rs`:

```rust
pub fn command_json(name: &str, value: serde_json::Value) -> Tick {
    Tick::Command(AppCommand::new(name, bytes::Bytes::from(value.to_string())))
}
```

- [ ] **Step 2: Implement retry command parser**

Create `crates/helix-im/src/send/retry_upload.rs`:

```rust
use crate::send::upload_props::{file_upload_request, UploadTarget};
use crate::state::TemporaryId;
use crate::{ImError, ImModule};
use helix_core::{Effect, EffectSink};
use serde_json::Value;

pub(crate) fn handle_retry_upload(
    module: &mut ImModule,
    payload: &[u8],
    out: &mut EffectSink,
) -> Result<(), ImError> {
    let cmd: Value = serde_json::from_slice(payload)
        .map_err(|e| ImError::Parse(format!("im_retry_upload payload: {e}")))?;
    let temporary_id = cmd["temporary_id"]
        .as_str()
        .ok_or_else(|| ImError::Parse("im_retry_upload missing temporary_id".to_string()))?;
    let target = parse_target(&cmd["target"])?;
    let tmp = TemporaryId(temporary_id.to_string());
    let body = module
        .state
        .pending_sends
        .get(&tmp)
        .and_then(|p| p.body.clone())
        .ok_or_else(|| ImError::Parse("im_retry_upload missing pending send".to_string()))?;

    let req = file_upload_request(&body["props"], &target)?;
    let corr = module.alloc_corr_internal();
    module.state.pending_uploads.insert(
        corr,
        crate::send::upload_props::PendingUpload {
            temporary_id: tmp,
            target,
        },
    );
    out.push(Effect::UploadFile { corr, req });
    Ok(())
}

fn parse_target(value: &Value) -> Result<UploadTarget, ImError> {
    match value["kind"].as_str() {
        Some("rich_image") => {
            let index = value["index"]
                .as_u64()
                .ok_or_else(|| ImError::Parse("rich_image retry requires index".to_string()))?;
            Ok(UploadTarget::RichImage { index: index as usize })
        }
        Some("file") => Ok(UploadTarget::File),
        _ => Err(ImError::Parse("unknown upload retry target".to_string())),
    }
}
```

- [ ] **Step 3: Route command**

Modify `crates/helix-im/src/send/mod.rs`:

```rust
pub mod retry_upload;
```

Modify `crates/helix-im/src/module.rs` command match:

```rust
"im_retry_upload" => {
    crate::send::retry_upload::handle_retry_upload(self, cmd.payload.as_ref(), out)
        .map_err(|e| CoreError::ModuleError {
            module: self.name(),
            source: Box::new(e),
        })?;
}
```

- [ ] **Step 4: Run focused tests**

Run:

```bash
cargo test -p helix-im retry_failed_image_emits_one_upload_only -- --nocapture
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/helix-im/src/send/retry_upload.rs crates/helix-im/src/send/mod.rs crates/helix-im/src/module.rs crates/helix-im/tests crates/helix-im/tests/common.rs
git commit -m "feat(im): retry failed rich/file uploads"
```

---

### Task 5: Cross-Crate Verification And Documentation Sync

**Files:**
- Modify: `docs/superpowers/specs/2026-07-06-rich-file-message-upload-design.md` only if implementation discovers a spec mismatch.
- Modify: `SESSION.md`

**Interfaces:**
- Consumes completed Tasks 1-4.
- Produces verified implementation ready for review.

- [ ] **Step 1: Run focused test suite**

Run:

```bash
cargo test -p helix-im rich_file_upload_contract -- --nocapture
cargo test -p helix-core upload_file_effect_carries_path_not_bytes
cargo check -p helix-driver-host
```

Expected: all PASS.

- [ ] **Step 2: Run dual-end product check**

Run:

```bash
cargo check -p helix-core -p helix-im -p helix-driver-native -p helix-driver-ffi
```

Expected: PASS.

- [ ] **Step 3: Run runtime-agnostic core gate**

Run:

```bash
cargo tree -p helix-core | grep -iE 'tokio|sqlx|reqwest|web-sys|tauri'
```

Expected: no output.

- [ ] **Step 4: Run formatting and diff checks**

Run:

```bash
cargo fmt --check
git diff --check
```

Expected: both PASS.

- [ ] **Step 5: Update SESSION**

Add a `✅ DONE` line to `SESSION.md` with:

```text
✅ DONE rich/file 消息上传发送实现 @<timestamp> | commits <range> | rich/file props 校验 + UploadFile effect/driver + send flow 上传门控 + retry_upload | 验证：<commands and PASS summary>
```

- [ ] **Step 6: Commit final docs/session update**

```bash
git add SESSION.md docs/superpowers/specs/2026-07-06-rich-file-message-upload-design.md
git commit -m "docs: 记录 rich/file 上传发送验收"
```

---

## Plan Self-Review

- Spec coverage: rich/file type rules covered by Task 1; props preservation covered by Task 1; UploadFile effect and driver boundary covered by Task 2; upload-gated send flow covered by Task 3; retry and no cancellation covered by Task 4; verification covered by Task 5.
- Placeholder scan: no placeholder work remains in this plan; every task has explicit paths, interfaces, commands, and expected results.
- Type consistency: `UploadTarget`, `UploadPlan`, `UploadSuccess`, `PendingUpload`, `FileUploadRequest`, and `Effect::UploadFile` are introduced before later tasks consume them.
