use bytes::Bytes;

use super::{FileUploadProgressPolicy, FileUploadUrls};

/// HTTP 请求（driver 的 HttpRequester 解析并发出）
#[derive(Debug, Clone)]
pub struct HttpRequest {
    pub method: String, // "GET" / "POST" / etc.
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<Bytes>,
}

/// HTTP 响应（driver 包装后通过 PortReply 回传）
#[derive(Debug, Clone)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Bytes,
}

/// 本地文件上传请求（driver 读取本地路径并执行真实上传）。
#[derive(Debug, Clone)]
pub struct FileUploadRequest {
    pub local_path: String,
    pub object_key: String,
    pub method: String,
    pub urls: FileUploadUrls,
    pub headers: Vec<(String, String)>,
    pub content_type: Option<String>,
    pub size: Option<u64>,
}

impl FileUploadRequest {
    /// 进度策略随已有私有 URL 封装携带，避免破坏公开结构体的字段级构造兼容。
    pub fn progress_policy(&self) -> FileUploadProgressPolicy {
        self.urls.progress_policy()
    }
}

/// 文件上传结果（driver 包装后通过 PortReply 回传）。
#[derive(Debug, Clone)]
pub struct FileUploadResponse {
    pub object_key: String,
    pub public_url: String,
    pub etag: Option<String>,
}
