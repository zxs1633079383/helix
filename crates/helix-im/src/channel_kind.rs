//! Channel wire type -> render-ready kind mapping.

use serde_json::Value;

pub(crate) fn channel_type(value: &Value) -> &str {
    value
        .as_object()
        .map(channel_type_from_object)
        .unwrap_or_default()
}

pub(crate) fn channel_type_from_object(value: &serde_json::Map<String, Value>) -> &str {
    value
        .get("channelType")
        .or_else(|| value.get("type"))
        .and_then(Value::as_str)
        .unwrap_or_default()
}

pub(crate) fn channel_kind(raw: &str) -> &'static str {
    match raw {
        "T" => "topic",
        "O" => "company",
        "D" => "direct",
        "P" => "group",
        _ => "group",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_all_supported_channel_types_without_collapsing_company() {
        assert_eq!(channel_kind("T"), "topic");
        assert_eq!(channel_kind("O"), "company");
        assert_eq!(channel_kind("P"), "group");
        assert_eq!(channel_kind("D"), "direct");
        assert_eq!(channel_kind(""), "group");
    }
}
