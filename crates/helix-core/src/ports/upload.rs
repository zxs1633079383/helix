//! FileUploader port — 本地文件上传抽象。
//!
//! 请求中的 `upload_url` 仅供 adapter 执行写入；成功结果只允许回传长期可读的
//! `public_url`，不得从预签名 PUT URL 推导或复制。

use crate::effect::{FileUploadRequest, FileUploadResponse};
use crate::error::PortError;
use crate::platform::{MaybeSend, MaybeSync};

/// 文件上传的短期写地址与长期读地址，二者必须同时存在。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileUploadUrls {
    upload_url: String,
    public_url: String,
}

impl FileUploadUrls {
    pub fn new(upload_url: String, public_url: String) -> Result<Self, &'static str> {
        if upload_url.trim().is_empty() || public_url.trim().is_empty() {
            return Err("upload_url and public_url must not be empty");
        }
        Ok(Self {
            upload_url,
            public_url,
        })
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
            .expect("valid urls"),
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
    }

    #[test]
    fn upload_urls_require_write_and_public_addresses() {
        assert!(FileUploadUrls::new(String::new(), "https://cdn/a".into()).is_err());
        assert!(FileUploadUrls::new("https://upload/a".into(), String::new()).is_err());
    }
}
