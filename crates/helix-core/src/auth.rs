//! # auth.rs
//!
//! 出站鉴权**意图**契约（generic，非业务概念）。
//!
//! ## 为什么意图在 core、token 在 driver（HX-C001 / C6）
//!
//! core 是纯状态机，**绝不持有也绝不拼任何鉴权 token**（session cookie / `Bearer mp_` 等）。
//! 业务模块（helix-im）发出 `Effect::Http` 时，只在请求头里**标一个鉴权意图**——
//! 「这条请求该用哪类凭据」——具体 token 的解析与注入由 driver 横切层在出站时完成。
//!
//! 这样切分：
//! - core/helix-im 永不出现真鉴权 token 字面量（C6 闸门：token 头名/值只在 driver 横切层）。
//! - token 轮换 / 登录态 / 401 重登都收敛在 driver 一处（现网 `networkInterceptor` 的归属）。
//!
//! 意图头是 **generic HTTP 鉴权路由**（`bot` vs `session` 两类凭据），不是 IM 的
//! message/channel/temporaryId 等业务概念——故落 core 不破「core 零业务」不变量。

use crate::effect::HttpRequest;

/// 出站鉴权意图头名（业务模块标意图 / driver 横切层消费后 **strip 掉**，不出网）。
///
/// 非标准 `X-` 头：仅在 helix 进程内 core→driver 之间传递意图，driver 注入真 token 后删除，
/// **绝不**随请求发到服务端（服务端只认真凭据，不认此意图头）。
pub const AUTH_KIND_HEADER: &str = "X-Auth-Kind";

/// 鉴权意图（generic 两类凭据路由）。`Effect::Http` 的 header 里带
/// `X-Auth-Kind: <as_str>`；driver 横切层据此选注哪类 token。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AuthKind {
    /// 用户会话凭据（现网 session cookie / cookieId 身份头一类）。
    Session,
    /// 机器人凭据（现网 `Bearer mp_` 一类）。
    Bot,
}

impl AuthKind {
    /// 意图头的 wire 值（小写稳定串，driver 横切层按此匹配）。
    pub const fn as_str(self) -> &'static str {
        match self {
            AuthKind::Session => "session",
            AuthKind::Bot => "bot",
        }
    }

    /// 从意图头值解析（driver 横切层用；未知值 → None，零信任不臆测）。
    pub fn from_str(value: &str) -> Option<Self> {
        match value {
            "session" => Some(AuthKind::Session),
            "bot" => Some(AuthKind::Bot),
            _ => None,
        }
    }
}

/// Consume the internal auth-intent header and inject the driver-owned
/// credential selected by `resolve`.
///
/// The function is pure: token storage and rotation remain in platform
/// drivers, while native/FFI/Web share the exact strip/override semantics.
pub fn apply_auth_intent(
    mut request: HttpRequest,
    mut resolve: impl FnMut(AuthKind) -> Option<(String, String)>,
) -> (HttpRequest, Option<AuthKind>) {
    let kind = request
        .headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(AUTH_KIND_HEADER))
        .and_then(|(_, value)| AuthKind::from_str(value));

    request
        .headers
        .retain(|(name, _)| !name.eq_ignore_ascii_case(AUTH_KIND_HEADER));

    if let Some(kind) = kind {
        if let Some((auth_name, auth_value)) = resolve(kind) {
            request
                .headers
                .retain(|(name, _)| !name.eq_ignore_ascii_case(&auth_name));
            request.headers.push((auth_name, auth_value));
        }
    }

    (request, kind)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_kind_roundtrip() {
        for kind in [AuthKind::Session, AuthKind::Bot] {
            assert_eq!(AuthKind::from_str(kind.as_str()), Some(kind));
        }
    }

    #[test]
    fn auth_kind_unknown_is_none() {
        assert_eq!(AuthKind::from_str("admin"), None);
        assert_eq!(AuthKind::from_str(""), None);
    }

    #[test]
    fn auth_kind_header_is_intent_only_namespace() {
        // 非标准头：进程内意图，driver 注 token 后 strip，绝不出网。
        assert_eq!(AUTH_KIND_HEADER, "X-Auth-Kind");
    }

    #[test]
    fn auth_intent_is_stripped_and_resolved_credential_overrides_stale_value() {
        let request = HttpRequest {
            method: "GET".into(),
            url: "https://example.invalid".into(),
            headers: vec![
                (AUTH_KIND_HEADER.into(), "session".into()),
                ("cookieId".into(), "stale".into()),
                ("x-request-id".into(), "request-1".into()),
            ],
            body: None,
        };
        let (prepared, kind) = apply_auth_intent(request, |requested| {
            (requested == AuthKind::Session).then(|| ("cookieId".into(), "current".into()))
        });

        assert_eq!(kind, Some(AuthKind::Session));
        assert!(!prepared
            .headers
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case(AUTH_KIND_HEADER)));
        assert!(prepared
            .headers
            .iter()
            .any(|(name, value)| name == "cookieId" && value == "current"));
        assert!(prepared
            .headers
            .iter()
            .any(|(name, value)| name == "x-request-id" && value == "request-1"));
    }
}
