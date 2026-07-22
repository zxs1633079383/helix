use helix_im::{
    state::Seq,
    sync::session::{
        compare_committed_recovery, AuthorityHead, CommittedRecoveryHead, EventEnvelope, EventKind,
        PostFields, RecoveryComparison, RecoveryPhase, RecoverySession, SyncBatchFacts,
    },
};

fn channel_id(seed: char) -> helix_im::state::ChannelId {
    helix_im::state::ChannelId::from_str(&seed.to_string().repeat(26))
        .expect("test channel id is valid")
}

fn committed(seq: u64) -> CommittedRecoveryHead {
    CommittedRecoveryHead {
        cursor: Seq(seq),
        ledger_to_seq: Seq(seq),
        coverage_to_seq: Seq(seq),
    }
}

#[test]
fn gs01_reconnect_compares_committed_evidence_before_any_recovery_output() {
    assert_eq!(
        compare_committed_recovery(committed(8), AuthorityHead { event_seq: Seq(11) }),
        RecoveryComparison::Pull {
            from_exclusive: Seq(8),
            to_inclusive: Seq(11),
        }
    );
    assert_eq!(
        compare_committed_recovery(committed(11), AuthorityHead { event_seq: Seq(11) }),
        RecoveryComparison::Equal
    );
}

#[test]
fn gs01_local_ahead_or_incoherent_evidence_requires_authority_reload() {
    assert_eq!(
        compare_committed_recovery(committed(12), AuthorityHead { event_seq: Seq(11) }),
        RecoveryComparison::AuthorityReloadRequired
    );
    assert_eq!(
        compare_committed_recovery(
            CommittedRecoveryHead {
                cursor: Seq(8),
                ledger_to_seq: Seq(7),
                coverage_to_seq: Seq(8),
            },
            AuthorityHead { event_seq: Seq(11) },
        ),
        RecoveryComparison::AuthorityReloadRequired
    );
}

#[test]
fn gs01_reconnect_equal_and_persist_failure_replay_has_no_false_completion() {
    let channel = channel_id('a');
    let mut session = RecoverySession::default();
    session.begin("actor-a");
    assert_eq!(session.phase, RecoveryPhase::Comparing);

    assert_eq!(
        session.compare(committed(11), AuthorityHead { event_seq: Seq(11) }),
        RecoveryComparison::Equal
    );
    assert_eq!(session.phase, RecoveryPhase::Recovered);

    session.await_commit(channel, Seq(12));
    session.commit_failed(channel);
    assert_eq!(session.phase, RecoveryPhase::Failed);
    assert!(
        !session.commit_ok(channel, Seq(12)),
        "a failed PersistAtomic cannot later manufacture a recovery completion"
    );
}

#[test]
fn gs02_accepts_viewer_filtered_gaps_but_rejects_non_increasing_evidence() {
    let channel = channel_id('b');
    let first = EventEnvelope::new(
        channel,
        Seq(8),
        EventKind::PostUpsert,
        PostFields::default(),
    );
    let visible_after_gap = EventEnvelope::new(
        channel,
        Seq(10),
        EventKind::PostUpsert,
        PostFields::default(),
    );
    assert!(SyncBatchFacts::from_events(
        channel,
        Seq(7),
        Seq(10),
        vec![first.clone(), visible_after_gap.clone()]
    )
    .is_ok());
    assert!(matches!(
        SyncBatchFacts::from_events(channel, Seq(7), Seq(10), vec![visible_after_gap, first]),
        Err("sync batch event sequence is not strictly increasing")
    ));
}

#[test]
fn gs03_unknown_channel_has_no_ready_composer_before_its_commit_boundary() {
    let channel = channel_id('c');
    let mut session = RecoverySession::default();
    session.begin("actor-a");
    session.await_commit(channel, Seq(1));
    assert_eq!(session.phase, RecoveryPhase::AwaitingCommit);
    assert!(session.commit_ok(channel, Seq(1)));
    assert_eq!(session.phase, RecoveryPhase::Recovered);
}

#[test]
fn gs04_relogin_rejects_a_session_late_persist_reply() {
    let channel = channel_id('d');
    let mut session = RecoverySession::default();
    session.begin("actor-a");
    session.await_commit(channel, Seq(9));
    let actor_a_epoch = session.session_epoch;
    session.invalidate();
    session.begin("actor-b");
    assert!(session.session_epoch > actor_a_epoch);
    assert!(!session.commit_ok(channel, Seq(9)));
    assert!(session.is_active_for("actor-b"));
    assert!(!session.is_active_for("actor-a"));
}
