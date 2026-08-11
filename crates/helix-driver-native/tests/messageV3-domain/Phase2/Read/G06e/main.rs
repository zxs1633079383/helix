use crate::shared;

/// MV3-G06e：未读角标由 Angular 从 Helix channel.unreadCount 派生，不拥有后端副作用。
#[tokio::test]
async fn mv3_g06e_real_chain() {
    shared::record_not_applicable(
        "Phase2",
        "Read",
        "MV3-G06e",
        "Angular-derived unread aggregate; Helix/Go does not own a separate command or persistence mutation.",
    )
    .unwrap_or_else(|error| panic!("MV3-G06e evidence failed: {error}"));
}
