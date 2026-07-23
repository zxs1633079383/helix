//! FileUploader port — 本地文件上传抽象。
//!
//! 请求中的 `upload_url` 仅供 adapter 执行写入；成功结果只允许回传长期可读的
//! `public_url`，不得从预签名 PUT URL 推导或复制。

use std::sync::Arc;

use crate::effect::{FileUploadRequest, FileUploadResponse};
use crate::error::PortError;
use crate::platform::{MaybeSend, MaybeSync};

/// 上传进度采样。driver 只报告已读出的字节事实；百分比节流由 host adapter 兑现。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileUploadProgress {
    pub completed_bytes: u64,
    pub total_bytes: u64,
}

/// 上传进度策略。`Disabled` 是默认值，避免图片等无需百分比的上传制造逐块 Tick。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FileUploadProgressPolicy {
    #[default]
    Disabled,
    /// 每跨过一个百分比步长才回灌。adapter 必须忽略 0 或大于 100 的非法步长。
    PercentStep(u8),
}

/// runtime-neutral 的上传进度回报口。
///
/// 具体 channel / atomic / task 均留在 driver；core 只定义同步、无 I/O 的字节事实回调。
pub trait FileUploadProgressReporter: MaybeSend + MaybeSync + 'static {
    fn report(&self, progress: FileUploadProgress);
}

/// 文件上传的短期写地址与长期读地址，二者必须同时存在。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileUploadUrls {
    upload_url: String,
    public_url: String,
    // 保持 FileUploadRequest 的公开字段兼容：策略随其已有私有封装携带。
    progress_policy: FileUploadProgressPolicy,
}

impl FileUploadUrls {
    pub fn new(upload_url: String, public_url: String) -> Result<Self, &'static str> {
        if upload_url.trim().is_empty() || public_url.trim().is_empty() {
            return Err("upload_url and public_url must not be empty");
        }
        Ok(Self {
            upload_url,
            public_url,
            progress_policy: FileUploadProgressPolicy::Disabled,
        })
    }

    pub fn with_progress_policy(mut self, policy: FileUploadProgressPolicy) -> Self {
        self.progress_policy = policy;
        self
    }

    pub fn progress_policy(&self) -> FileUploadProgressPolicy {
        self.progress_policy
    }

    pub fn upload_url(&self) -> &str {
        &self.upload_url
    }

    pub fn public_url(&self) -> &str {
        &self.public_url
    }

    pub fn into_parts(self) -> (String, String) {
        (self.upload_url, self.public_url)
    }
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
pub trait FileUploader: MaybeSend + MaybeSync + 'static {
    async fn upload(&self, req: FileUploadRequest) -> Result<FileUploadResponse, PortError>;

    /// 带可选进度回报的上传入口。既有 adapter 无需改动即可沿用终态-only 行为。
    async fn upload_with_progress(
        &self,
        req: FileUploadRequest,
        progress: Option<Arc<dyn FileUploadProgressReporter>>,
    ) -> Result<FileUploadResponse, PortError> {
        let _ = progress;
        self.upload(req).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effect::{Correlation, Effect, FileUploadRequest};

    #[test]
    fn upload_effect_keeps_distinct_validated_urls() {
        let req = FileUploadRequest {
            local_path: "/tmp/a.png".into(),
            object_key: "im/a.png".into(),
            method: "PUT".into(),
            urls: FileUploadUrls::new(
                "https://upload/a?signature=secret".into(),
                "https://cdn/a.png".into(),
            )
            .expect("valid urls")
            .with_progress_policy(FileUploadProgressPolicy::PercentStep(5)),
            headers: Vec::new(),
            content_type: Some("image/png".into()),
            size: Some(123),
        };
        let Effect::UploadFile { corr, req } = (Effect::UploadFile {
            corr: Correlation::from_raw(9),
            req,
        }) else {
            panic!("expected upload effect");
        };
        assert_eq!(corr.raw(), 9);
        assert!(req.urls.upload_url().contains("signature=secret"));
        assert_eq!(req.urls.public_url(), "https://cdn/a.png");
        assert_eq!(
            req.progress_policy(),
            FileUploadProgressPolicy::PercentStep(5)
        );
    }

    #[test]
    fn upload_urls_require_write_and_public_addresses() {
        assert!(FileUploadUrls::new(String::new(), "https://cdn/a".into()).is_err());
        assert!(FileUploadUrls::new("https://upload/a".into(), String::new()).is_err());
    }
}
