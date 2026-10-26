pub const MAX_METRIC_LABELS: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LabelKey {
    Layer,
    Scenario,
    Stage,
    Action,
    TickKind,
    EffectKind,
    StorageOp,
    Platform,
    ChannelShape,
    PayloadBucket,
    RateBucket,
    Status,
    ServiceName,
    DeploymentEnvironment,
    Protocol,
    Operation,
    Direction,
    StatusClass,
    ErrorKind,
    ExporterState,
    Pool,
    Consumer,
    Overflow,
    LifecycleState,
    Terminal,
    Path,
    Role,
    Result,
    RecipientScope,
    State,
}

impl LabelKey {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Layer => "layer",
            Self::Scenario => "scenario",
            Self::Stage => "stage",
            Self::Action => "action",
            Self::TickKind => "tick_kind",
            Self::EffectKind => "effect_kind",
            Self::StorageOp => "storage_op",
            Self::Platform => "platform",
            Self::ChannelShape => "channel_shape",
            Self::PayloadBucket => "payload_bucket",
            Self::RateBucket => "rate_bucket",
            Self::Status => "status",
            Self::ServiceName => "service_name",
            Self::DeploymentEnvironment => "deployment_environment",
            Self::Protocol => "protocol",
            Self::Operation => "operation",
            Self::Direction => "direction",
            Self::StatusClass => "status_class",
            Self::ErrorKind => "error_kind",
            Self::ExporterState => "exporter_state",
            Self::Pool => "pool",
            Self::Consumer => "consumer",
            Self::Overflow => "overflow",
            Self::LifecycleState => "lifecycle_state",
            Self::Terminal => "terminal",
            Self::Path => "path",
            Self::Role => "role",
            Self::Result => "result",
            Self::RecipientScope => "recipient_scope",
            Self::State => "state",
        }
    }
}

pub const ALLOWED_LABEL_KEYS: &[LabelKey] = &[
    LabelKey::Layer,
    LabelKey::Scenario,
    LabelKey::Stage,
    LabelKey::Action,
    LabelKey::TickKind,
    LabelKey::EffectKind,
    LabelKey::StorageOp,
    LabelKey::Platform,
    LabelKey::ChannelShape,
    LabelKey::PayloadBucket,
    LabelKey::RateBucket,
    LabelKey::Status,
    LabelKey::ServiceName,
    LabelKey::DeploymentEnvironment,
    LabelKey::Protocol,
    LabelKey::Operation,
    LabelKey::Direction,
    LabelKey::StatusClass,
    LabelKey::ErrorKind,
    LabelKey::ExporterState,
    LabelKey::Pool,
    LabelKey::Consumer,
    LabelKey::Overflow,
    LabelKey::LifecycleState,
    LabelKey::Terminal,
    LabelKey::Path,
    LabelKey::Role,
    LabelKey::Result,
    LabelKey::RecipientScope,
    LabelKey::State,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MetricLabel {
    pub key: LabelKey,
    pub value: &'static str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MetricLabels {
    values: [Option<MetricLabel>; MAX_METRIC_LABELS],
    len: u8,
}

impl MetricLabels {
    pub const EMPTY: Self = Self {
        values: [None; MAX_METRIC_LABELS],
        len: 0,
    };

    pub const fn one(key: LabelKey, value: &'static str) -> Self {
        let mut labels = Self::EMPTY;
        labels.values[0] = Some(MetricLabel { key, value });
        labels.len = 1;
        labels
    }

    pub fn with(mut self, key: LabelKey, value: &'static str) -> Self {
        let index = self.len as usize;
        if index < MAX_METRIC_LABELS {
            self.values[index] = Some(MetricLabel { key, value });
            self.len += 1;
        }
        self
    }

    pub fn iter(&self) -> impl Iterator<Item = MetricLabel> + '_ {
        self.values[..self.len as usize].iter().flatten().copied()
    }

    pub const fn len(&self) -> usize {
        self.len as usize
    }

    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl Default for MetricLabels {
    fn default() -> Self {
        Self::EMPTY
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whitelist_is_exact_and_excludes_high_cardinality_keys() {
        assert_eq!(ALLOWED_LABEL_KEYS.len(), 30);
        let names: Vec<_> = ALLOWED_LABEL_KEYS.iter().map(|key| key.as_str()).collect();
        for forbidden in [
            "runId",
            "corr_key",
            "req_id",
            "temporaryId",
            "postId",
            "channelId",
            "userId",
            "eventSeq",
        ] {
            assert!(!names.contains(&forbidden));
        }
    }
}
