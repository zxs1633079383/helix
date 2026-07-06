use crate::module::ImModule;
use crate::send::upload_props::{file_upload_request, PendingUpload, UploadTarget};
use crate::state::TemporaryId;
use crate::ImError;
use helix_core::{Effect, EffectSink};
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
struct RetryUploadCommand {
    temporary_id: String,
    target: RetryUploadTarget,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum RetryUploadTarget {
    RichImage { index: usize },
    File,
}

impl RetryUploadTarget {
    fn into_upload_target(self) -> UploadTarget {
        match self {
            Self::RichImage { index } => UploadTarget::RichImage { index },
            Self::File => UploadTarget::File,
        }
    }
}

pub(crate) fn handle_retry_upload(
    module: &mut ImModule,
    payload: &[u8],
    out: &mut EffectSink,
) -> Result<(), ImError> {
    let cmd: RetryUploadCommand = serde_json::from_slice(payload)
        .map_err(|e| ImError::Parse(format!("im_retry_upload payload: {e}")))?;
    let temporary_id = TemporaryId(cmd.temporary_id);
    let target = cmd.target.into_upload_target();

    let req = {
        let pending_send = module
            .state
            .pending_sends
            .get_mut(&temporary_id)
            .ok_or_else(|| ImError::Parse("im_retry_upload missing pending send".to_string()))?;
        let body = pending_send.body.as_mut().ok_or_else(|| {
            ImError::Parse("im_retry_upload missing cached send body".to_string())
        })?;
        let props = body
            .get_mut("props")
            .ok_or_else(|| ImError::Parse("im_retry_upload missing props".to_string()))?;

        ensure_target_failed(props, &target)?;
        set_target_status(props, &target, "uploading")?;
        pending_send.upload_failed = has_failed_target(props);

        file_upload_request(props, &target)?
    };

    let corr = module.alloc_corr_internal();
    module.state.pending_uploads.insert(
        corr,
        PendingUpload {
            temporary_id,
            target,
        },
    );
    out.push(Effect::UploadFile { corr, req });
    Ok(())
}

fn ensure_target_failed(props: &Value, target: &UploadTarget) -> Result<(), ImError> {
    let status = target_status(props, target)?;
    if status == "failed" {
        Ok(())
    } else {
        Err(ImError::Parse(format!(
            "upload retry target is not failed: {status}"
        )))
    }
}

fn target_status<'a>(props: &'a Value, target: &UploadTarget) -> Result<&'a str, ImError> {
    target_upload(props, target)?
        .get("status")
        .and_then(Value::as_str)
        .ok_or_else(|| ImError::Parse("upload retry target missing upload.status".to_string()))
}

fn set_target_status(
    props: &mut Value,
    target: &UploadTarget,
    status: &str,
) -> Result<(), ImError> {
    target_upload_mut(props, target)?
        .insert("status".to_string(), Value::String(status.to_string()));
    Ok(())
}

fn has_failed_target(props: &Value) -> bool {
    props
        .get("images")
        .and_then(Value::as_array)
        .map(|images| images.iter().any(node_failed))
        .unwrap_or(false)
        || props.get("file").map(node_failed).unwrap_or(false)
}

fn node_failed(node: &Value) -> bool {
    node.get("upload")
        .and_then(Value::as_object)
        .and_then(|upload| upload.get("status"))
        .and_then(Value::as_str)
        == Some("failed")
}

fn target_upload<'a>(
    props: &'a Value,
    target: &UploadTarget,
) -> Result<&'a serde_json::Map<String, Value>, ImError> {
    target_node(props, target)?
        .get("upload")
        .and_then(Value::as_object)
        .ok_or_else(|| ImError::Parse("upload retry target missing upload".to_string()))
}

fn target_upload_mut<'a>(
    props: &'a mut Value,
    target: &UploadTarget,
) -> Result<&'a mut serde_json::Map<String, Value>, ImError> {
    target_node_mut(props, target)?
        .get_mut("upload")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| ImError::Parse("upload retry target missing upload".to_string()))
}

fn target_node<'a>(props: &'a Value, target: &UploadTarget) -> Result<&'a Value, ImError> {
    match target {
        UploadTarget::RichImage { index } => props
            .get("images")
            .and_then(Value::as_array)
            .and_then(|images| images.get(*index))
            .ok_or_else(|| ImError::Parse("missing rich image retry target".to_string())),
        UploadTarget::File => props
            .get("file")
            .ok_or_else(|| ImError::Parse("missing file retry target".to_string())),
    }
}

fn target_node_mut<'a>(
    props: &'a mut Value,
    target: &UploadTarget,
) -> Result<&'a mut Value, ImError> {
    match target {
        UploadTarget::RichImage { index } => props
            .get_mut("images")
            .and_then(Value::as_array_mut)
            .and_then(|images| images.get_mut(*index))
            .ok_or_else(|| ImError::Parse("missing rich image retry target".to_string())),
        UploadTarget::File => props
            .get_mut("file")
            .ok_or_else(|| ImError::Parse("missing file retry target".to_string())),
    }
}
