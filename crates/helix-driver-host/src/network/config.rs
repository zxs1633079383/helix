use std::sync::Arc;
use std::time::Duration;

use helix_core::PortError;
use parking_lot::RwLock;
use reqwest::header::HeaderMap;

use super::network_util::{header_name, header_value};

const DEFAULT_TIMEOUT_SECS: u64 = 30;

/// Platform-provided network initialization material.
#[derive(Debug, Clone)]
pub struct HostNetworkConfig {
    /// Go IM base URL（例如 `/api/cses` 网关）。
    pub api_base_url: String,
    /// 前端/平台显式提供的 Java 默认业务网关 base URL；空串表示未注入。
    ///
    /// 不允许从 `api_base_url` 裁路径或拼域名推测，两者可以是完全不同的 origin。
    pub default_api_base_url: String,
    pub ws_url: String,
    pub timeout: Duration,
    pub user_agent: Option<String>,
}

impl HostNetworkConfig {
    pub fn new(api_base_url: impl Into<String>, ws_url: impl Into<String>) -> Self {
        Self {
            api_base_url: api_base_url.into(),
            default_api_base_url: String::new(),
            ws_url: ws_url.into(),
            timeout: Duration::from_secs(DEFAULT_TIMEOUT_SECS),
            user_agent: None,
        }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn with_user_agent(mut self, user_agent: impl Into<String>) -> Self {
        self.user_agent = Some(user_agent.into());
        self
    }

    /// 显式注入 Java/default business gateway；不会从 Go base 派生。
    pub fn with_default_api_base_url(mut self, api_base_url: impl Into<String>) -> Self {
        self.default_api_base_url = api_base_url.into();
        self
    }
}

/// Shared mutable header registry used by HTTP requests and WS handshakes.
#[derive(Clone, Default)]
pub struct HostHeaderRegistry {
    inner: Arc<RwLock<HeaderMap>>,
}

impl HostHeaderRegistry {
    pub async fn set_header(&self, name: &str, value: &str) -> Result<(), PortError> {
        self.set_header_sync(name, value)
    }

    pub fn set_header_sync(&self, name: &str, value: &str) -> Result<(), PortError> {
        let name = header_name(name)?;
        let value = header_value(name.as_str(), value)?;
        self.inner.write().insert(name, value);
        Ok(())
    }

    pub async fn remove_header(&self, name: &str) -> Result<(), PortError> {
        self.remove_header_sync(name)
    }

    pub fn remove_header_sync(&self, name: &str) -> Result<(), PortError> {
        let name = header_name(name)?;
        self.inner.write().remove(name);
        Ok(())
    }

    pub async fn replace_headers<I, K, V>(&self, headers: I) -> Result<(), PortError>
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<str>,
        V: AsRef<str>,
    {
        let mut next = HeaderMap::new();
        for (name, value) in headers {
            let name = header_name(name.as_ref())?;
            let value = header_value(name.as_str(), value.as_ref())?;
            next.insert(name, value);
        }
        *self.inner.write() = next;
        Ok(())
    }

    pub async fn snapshot(&self) -> HeaderMap {
        self.inner.read().clone()
    }
}
