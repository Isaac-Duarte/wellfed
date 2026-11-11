//! Phone keys against real SQLite: a key asked for again keeps its
//! token's record, the reconcile marks a key whose account or token no
//! longer may hold one, and a key a phone key invited is recorded as that
//! key's account's and goes with it.

use jiff::Timestamp;
use tempfile::TempDir;
use uuid::Uuid;
use wellfed_api::{
    ClientRole, CoreEvent, EventBody, FeederId, Hex, Home, HomeId, Model, RevokeReason, TimeFormat,
    TokenId, Units, UserId,
};
use wellfed_store::{NewEvent, NewFeeder, NewPhone, Recorded, SealedKey, Store, Whose};

fn at(ms: i64) -> Timestamp {
    Timestamp::from_millisecond(ms).unwrap()
}

async fn fresh() -> (TempDir, Store, FeederId) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("server.db"), at(0))
        .await
        .unwrap();
    home(&store).await;
    let feeder = FeederId(Uuid::from_u128(1));
    store
        .insert_feeder(NewFeeder {
            id: feeder,
            home: HOME,
            model: Model::Plaf103,
            identity: [1; 32],
            name: "kitchen",
            role: ClientRole::Owner,
            address: None,
            key: SealedKey {
                nonce: [7; 12],
                sealed: vec![1, 2, 3],
            },
            created: at(1),
        })
        .await
        .unwrap();
    (dir, store, feeder)
}

fn phone(feeder: FeederId, key: u8, token: u128, until: i64) -> NewPhone {
    NewPhone {
        feeder,
        key: [key; 32],
        user: UserId(Uuid::from_u128(0x50)),
        token: TokenId(Uuid::from_u128(token)),
        role: ClientRole::Member,
        invited: at(10),
        invite_until: at(until),
    }
}

#[tokio::test]
async fn a_key_asked_for_again_raises_its_deadline_and_another_tokens_is_taken() {
    let (_dir, store, feeder) = fresh().await;

    assert_eq!(
        store.record_phone(&phone(feeder, 1, 7, 100)).await.unwrap(),
        Recorded::New
    );
    assert_eq!(
        store.record_phone(&phone(feeder, 1, 7, 50)).await.unwrap(),
        Recorded::Again { until: at(100) }
    );
    let kept = store.phone(feeder, &[1; 32]).await.unwrap().unwrap();
    assert_eq!(kept.invite_until, at(100), "never lowered");
    assert_eq!(
        store.record_phone(&phone(feeder, 1, 8, 200)).await.unwrap(),
        Recorded::Taken
    );

    let stale = store
        .settle_invite_until(feeder, &[1; 32], at(50), at(60))
        .await
        .unwrap();
    assert!(!stale, "another request's bound is not lowered");
    let settled = store
        .settle_invite_until(feeder, &[1; 32], at(100), at(60))
        .await
        .unwrap();
    assert!(settled);
    let set = store.phone(feeder, &[1; 32]).await.unwrap().unwrap();
    assert_eq!(set.invite_until, at(60));
}

/// The key's account and token were never stored (or are gone): the
/// reconcile marks it as a deleted account's, once.
#[tokio::test]
async fn the_reconcile_marks_a_key_whose_account_is_gone() {
    let (_dir, store, feeder) = fresh().await;
    store.record_phone(&phone(feeder, 2, 7, 100)).await.unwrap();

    assert_eq!(store.reconcile_phones(at(20)).await.unwrap(), [feeder]);

    let marked = store.phone(feeder, &[2; 32]).await.unwrap().unwrap();
    let revoking = marked.revoking.unwrap();
    assert_eq!(
        (revoking.reason, revoking.since),
        (RevokeReason::Deleted, at(20))
    );
    assert!(store.reconcile_phones(at(30)).await.unwrap().is_empty());
}

/// The feeder reports that key `invitee` joined through key `inviter`'s
/// invite, as event `seq`, read after every key here was recorded.
async fn joined(store: &Store, feeder: FeederId, seq: u64, invitee: u8, inviter: u8) {
    joined_at(store, feeder, (seq, 30), invitee, inviter).await;
}

/// As [`joined`], the event `seq` read at `received`.
async fn joined_at(
    store: &Store,
    feeder: FeederId,
    (seq, received): (u64, i64),
    invitee: u8,
    inviter: u8,
) {
    let body = EventBody::Core(CoreEvent::Invited {
        client: Hex([invitee; 7]),
        by: Hex([inviter; 7]),
    });
    let event = NewEvent {
        epoch: 1,
        seq,
        at: None,
        received: at(received),
        body: Some(body),
        raw: &[22],
    };
    assert!(store.record_event(feeder, event).await.unwrap());
}

/// Attribute every join a phone key made that the feeder reported, as a
/// revoke pass does, `listed` being its clients.
async fn attribute(store: &Store, feeder: FeederId, listed: &[u8], now: i64) -> bool {
    let mut owed = false;
    loop {
        let pending = store.invitations(feeder).await.unwrap();
        if pending.is_empty() {
            return owed;
        }
        for invitation in pending {
            let joined: Vec<_> = listed
                .iter()
                .filter(|&&n| [n; 7] == invitation.invitee)
                .map(|&n| ([n; 32], ClientRole::Member))
                .collect();
            owed |= store
                .attribute_invitation(&invitation, &joined, at(now))
                .await
                .unwrap();
        }
    }
}

fn reason(phone: Option<wellfed_api::PhoneKey>) -> Option<RevokeReason> {
    phone?.revoking.map(|revoking| revoking.reason)
}

/// A key a phone key invited is recorded under that key's account and
/// token, and a chain of invites goes when its first key's token does.
#[tokio::test]
async fn an_invitee_takes_its_inviters_account_and_goes_with_it() {
    let (_dir, store, feeder) = fresh().await;
    store.record_phone(&phone(feeder, 1, 7, 10)).await.unwrap();
    joined(&store, feeder, 1, 2, 1).await;
    joined(&store, feeder, 2, 3, 2).await;
    // A join no phone key invited (the server's own, an owner's CLI).
    joined(&store, feeder, 3, 4, 9).await;

    assert!(!attribute(&store, feeder, &[1, 2, 3, 4], 40).await);
    assert!(store.invitations(feeder).await.unwrap().is_empty());
    for n in [2, 3] {
        let invitee = store.phone(feeder, &[n; 32]).await.unwrap().unwrap();
        assert_eq!(invitee.token, Some(TokenId(Uuid::from_u128(7))));
        assert!(invitee.revoking.is_none());
    }
    assert_eq!(store.phone(feeder, &[4; 32]).await.unwrap(), None);

    let token = Whose::Token(TokenId(Uuid::from_u128(7)));
    let feeders = store
        .revoke_phones(token, RevokeReason::TokenRevoked, at(50))
        .await
        .unwrap();
    assert_eq!(feeders, [feeder]);
    assert_eq!(store.revoking(feeder).await.unwrap().len(), 3);
}

/// Revoking the inviting key alone marks its invitees and theirs, and a
/// join read only after the inviter was dropped is owed a revoke as it is
/// recorded.
#[tokio::test]
async fn a_dropped_inviter_takes_its_invitees_however_late_they_are_read() {
    let (_dir, store, feeder) = fresh().await;
    store.record_phone(&phone(feeder, 1, 7, 10)).await.unwrap();
    joined(&store, feeder, 1, 2, 1).await;
    joined(&store, feeder, 2, 3, 2).await;
    assert!(!attribute(&store, feeder, &[1, 2, 3], 40).await);

    assert!(store.forget_client(feeder, &[1; 32], at(50)).await.unwrap());
    for n in [2, 3] {
        let invitee = store.phone(feeder, &[n; 32]).await.unwrap();
        assert_eq!(reason(invitee), Some(RevokeReason::InviterRevoked), "{n}");
    }

    // Key 1 invited key 5 too, which the server reads only now.
    joined(&store, feeder, 3, 5, 1).await;
    assert!(attribute(&store, feeder, &[2, 3, 5], 60).await);
    let late = store.phone(feeder, &[5; 32]).await.unwrap();
    assert_eq!(late.as_ref().unwrap().user, UserId(Uuid::from_u128(0x50)));
    assert_eq!(reason(late), Some(RevokeReason::InviterRevoked));
}

/// A join the server read before it recorded the inviting key is not
/// that key's: claiming a phone does not sweep in the invites it made
/// before. Its later joins are.
#[tokio::test]
async fn a_join_read_before_its_inviter_was_recorded_is_not_attributed() {
    let (_dir, store, feeder) = fresh().await;
    joined_at(&store, feeder, (1, 5), 2, 1).await;
    store.record_phone(&phone(feeder, 1, 7, 10)).await.unwrap();
    assert!(store.invitations(feeder).await.unwrap().is_empty());

    joined(&store, feeder, 2, 3, 1).await;
    assert!(!attribute(&store, feeder, &[1, 2, 3], 40).await);
    assert_eq!(store.phone(feeder, &[2; 32]).await.unwrap(), None);
    let later = store.phone(feeder, &[3; 32]).await.unwrap().unwrap();
    assert_eq!(later.token, Some(TokenId(Uuid::from_u128(7))));
    assert_eq!(
        later.invited,
        at(30),
        "recorded as of when its join was read"
    );

    // Nor is it pinned on the key once the key is dropped: only the later
    // join goes.
    assert!(store.forget_client(feeder, &[1; 32], at(50)).await.unwrap());
    let later = store.phone(feeder, &[3; 32]).await.unwrap();
    assert_eq!(reason(later), Some(RevokeReason::InviterRevoked));
    assert_eq!(store.phone(feeder, &[2; 32]).await.unwrap(), None);
    assert!(store.invitations(feeder).await.unwrap().is_empty());
}

/// A key an owner kept stays theirs, whoever invited it.
#[tokio::test]
async fn a_kept_invitee_is_not_attributed() {
    let (_dir, store, feeder) = fresh().await;
    store.record_phone(&phone(feeder, 1, 7, 10)).await.unwrap();
    store.keep_client(feeder, &[2; 32], at(20)).await.unwrap();
    joined(&store, feeder, 1, 2, 1).await;

    assert!(!attribute(&store, feeder, &[1, 2], 40).await);
    assert_eq!(store.phone(feeder, &[2; 32]).await.unwrap(), None);
    assert!(store.invitations(feeder).await.unwrap().is_empty());
}

/// The home every device here belongs to.
const HOME: HomeId = HomeId(Uuid::from_u128(0x40));

async fn home(store: &Store) {
    let home = Home {
        id: HOME,
        name: "Home".into(),
        units: Units::Grams,
        time_format: TimeFormat::H24,
        zone: "UTC".into(),
    };
    store
        .create_home(&home, jiff::Timestamp::UNIX_EPOCH)
        .await
        .unwrap();
}
