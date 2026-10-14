//! 登录身份决定本地查询范围；频道权限始终由成员事实与服务端裁决。

/// 有公司身份保留公司范围；个人身份只绑定登录用户，不伪造公司 ID。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AccountScope<'a> {
    Personal {
        user_id: &'a str,
    },
    Company {
        user_id: &'a str,
        company_id: &'a str,
    },
}

impl<'a> AccountScope<'a> {
    /// 在身份边界选择策略，调用者负责拒绝空 userId。
    pub(crate) fn new(user_id: &'a str, company_id: &'a str) -> Self {
        if company_id.is_empty() {
            Self::Personal { user_id }
        } else {
            Self::Company {
                user_id,
                company_id,
            }
        }
    }

    /// 本地数据按登录账号隔离；公司身份不能排除已授权的跨公司成员频道。
    pub(crate) fn channel_filter(self) -> (&'static str, &'a str) {
        match self {
            Self::Personal { user_id } => ("user_id", user_id),
            Self::Company { user_id, .. } => ("user_id", user_id),
        }
    }

    /// 群租户必须存在，访问资格由账号成员事实裁决，不按登录公司拒绝。
    pub(crate) fn accepts_channel_tenant(self, tenant: Option<&str>) -> bool {
        match self {
            Self::Personal { .. } => tenant.is_some_and(|value| !value.is_empty()),
            Self::Company { .. } => tenant.is_some_and(|value| !value.is_empty()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn personal_scope_uses_user_without_claiming_company() {
        let scope = AccountScope::new("a", "");
        assert_eq!(scope.channel_filter(), ("user_id", "a"));
        assert!(scope.accepts_channel_tenant(Some("b")));
        assert!(!scope.accepts_channel_tenant(None));
    }
    #[test]
    fn company_scope_keeps_account_isolation_without_company_restriction() {
        let scope = AccountScope::new("a", "company");
        assert_eq!(scope.channel_filter(), ("user_id", "a"));
        assert!(scope.accepts_channel_tenant(Some("company")));
        assert!(scope.accepts_channel_tenant(Some("other")));
        assert!(!scope.accepts_channel_tenant(Some("")));
    }
}
