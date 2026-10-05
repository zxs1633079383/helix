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

    /// 本地频道行的结构化过滤策略；个人模式按账户快照，不按公司过滤。
    pub(crate) fn channel_filter(self) -> (&'static str, &'a str) {
        match self {
            Self::Personal { user_id } => ("user_id", user_id),
            Self::Company { company_id, .. } => ("team_id", company_id),
        }
    }

    /// 公司模式保持原有边界，个人模式允许属于其他租户的频道，仍需后续成员校验。
    pub(crate) fn accepts_channel_tenant(self, tenant: Option<&str>) -> bool {
        match self {
            Self::Personal { .. } => tenant.is_some_and(|value| !value.is_empty()),
            Self::Company { company_id, .. } => tenant == Some(company_id),
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
    fn company_scope_keeps_existing_filter() {
        let scope = AccountScope::new("a", "company");
        assert_eq!(scope.channel_filter(), ("team_id", "company"));
        assert!(scope.accepts_channel_tenant(Some("company")));
        assert!(!scope.accepts_channel_tenant(Some("other")));
    }
}
