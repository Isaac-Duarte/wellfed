//! Several sessions following the events at once, through the turns the
//! firmware reads its event ring in: each gets every event of a feed, one
//! still reading the backlog included, and one that leaves mid-read stalls
//! nobody.

use std::time::Duration;
use wellfed_connect::Samples;

use wellfed_client::api::Invite;
use wellfed_connect::{Connection, Cursor, join};
use wellfed_feeder_sim::flash::Flash;
use wellfed_feeder_sim::harness::{Running, local};
use wellfed_model::{Answer, ClientRole, EventBody, FeederEvent, Op, Pushed, Status};
use wellfed_proto::api::RequestId;
use wellfed_session::noise::Keypair;

const OWNER: [u8; 32] = [0x21; 32];
const MEMBER: [u8; 32] = [0x31; 32];
const WAIT: Duration = Duration::from_secs(5);

/// More lock changes than two read batches hold, so a session subscribed
/// from the oldest event is still catching up while others are not.
const BACKLOG: u8 = 80;

fn keys(secret: [u8; 32]) -> Keypair {
    Keypair::from_secret(secret)
}

/// A simulator with its owner enrolled, and request ids for the test.
struct Rig {
    sim: Running,
    ids: u8,
}

impl Rig {
    async fn start() -> Self {
        let owner = Some(*keys(OWNER).public());
        Self {
            sim: Running::start(&Flash::memory(), owner).await,
            ids: 0,
        }
    }

    fn id(&mut self) -> RequestId {
        self.ids = self.ids.wrapping_add(1);
        [self.ids; 16]
    }

    async fn ask(&mut self, connection: &mut Connection, op: &Op) -> Answer {
        let id = self.id();
        connection.ask(id, op).await.unwrap()
    }

    /// A session as the owner, its ephemeral key told apart by `n`.
    async fn owner(&self, n: u8) -> Connection {
        Connection::open(&keys(OWNER), self.sim.identity, self.sim.address, [n; 32])
            .await
            .unwrap()
    }

    /// A member `owner` invited, joined and in session.
    async fn member(&mut self, owner: &mut Connection) -> Connection {
        let role = ClientRole::Member;
        let Answer::Invite(invite) = self.ask(owner, &Op::CreateInvite { role }).await else {
            panic!("an owner may invite");
        };
        let invite = Invite {
            feeder: self.sim.identity,
            id: invite.id,
            secret: invite.secret.0,
        };
        join(
            &keys(MEMBER),
            invite,
            "member",
            self.sim.address,
            [0x92; 32],
        )
        .await
        .unwrap();
        Connection::open(
            &keys(MEMBER),
            self.sim.identity,
            self.sim.address,
            [0x93; 32],
        )
        .await
        .unwrap()
    }

    async fn status(&mut self, connection: &mut Connection) -> Status {
        let Answer::Status(status) = self.ask(connection, &Op::Status).await else {
            panic!("status answers");
        };
        status
    }

    /// `BACKLOG` lock changes, each an event of its own.
    async fn fill_backlog(&mut self, owner: &mut Connection) {
        for n in 0..BACKLOG {
            let locked = n % 2 == 0;
            let answer = self.ask(owner, &Op::SetLock { locked }).await;
            assert!(matches!(answer, Answer::Done { .. }), "{answer:?}");
        }
    }

    /// One portion, by `connection`.
    async fn feed(&mut self, connection: &mut Connection) {
        let answer = self.ask(connection, &Op::Feed { portions: 1 }).await;
        assert!(matches!(answer, Answer::Feeding { .. }), "{answer:?}");
    }
}

/// Every event pushed to `connection` until the first `Fed` after `after`,
/// by sequence number. Fails on a gap, or on a number out of order.
async fn until_fed(connection: &mut Connection, after: Option<u64>) -> Vec<(u64, EventBody)> {
    let mut events: Vec<(u64, EventBody)> = Vec::new();
    loop {
        match connection.next(WAIT).await.unwrap() {
            Pushed::Event(event) => {
                if let Some(&(last, _)) = events.last() {
                    assert_eq!(event.seq, last + 1, "events arrive in order, none skipped");
                }
                let fed = matches!(event.body, EventBody::Feeder(FeederEvent::Fed { .. }));
                let new = after.is_none_or(|after| event.seq > after);
                events.push((event.seq, event.body));
                if fed && new {
                    return events;
                }
            }
            Pushed::Gap(gap) => panic!("events lost: {gap:?}"),
            Pushed::Unknown(event) => {
                panic!("this build recorded a body it cannot read: {event:?}")
            }
            Pushed::Sample(_) | Pushed::Log(_) => {}
        }
    }
}

/// A cursor at the newest event, as a session already caught up holds.
fn at_newest(connection: &Connection, newest: Option<u64>) -> Option<Cursor> {
    let seq = newest?;
    Some(Cursor {
        epoch: connection.epoch(),
        seq,
    })
}

/// The events after `after` in `events`.
fn after(events: &[(u64, EventBody)], after: Option<u64>) -> Vec<(u64, EventBody)> {
    events
        .iter()
        .filter(|(seq, _)| after.is_none_or(|after| *seq > after))
        .cloned()
        .collect()
}

#[test]
fn three_sessions_following_at_once_each_get_every_event_of_a_feed() {
    local(async {
        let mut rig = Rig::start().await;
        let mut owner = rig.owner(0xa1).await;
        let mut member = rig.member(&mut owner).await;
        let mut third = rig.owner(0xa2).await;

        let before = rig.status(&mut owner).await.last_event;
        for session in [&mut owner, &mut member, &mut third] {
            session.subscribe(None, Samples::Off).await.unwrap();
        }
        rig.feed(&mut member).await;

        let mut seen = Vec::new();
        for session in [&mut owner, &mut member, &mut third] {
            seen.push(after(&until_fed(session, before).await, before));
        }
        assert!(!seen[0].is_empty());
        assert_eq!(seen[0], seen[1], "the member saw what the owner saw");
        assert_eq!(seen[0], seen[2], "the third saw what the owner saw");

        rig.sim.stop().await;
    });
}

#[test]
fn a_session_still_reading_the_backlog_and_two_caught_up_each_get_the_feed() {
    local(async {
        let mut rig = Rig::start().await;
        let mut owner = rig.owner(0xa1).await;
        let mut member = rig.member(&mut owner).await;
        rig.fill_backlog(&mut owner).await;

        // Two follow from the newest event; the third replays the whole
        // ring, more than two batches of it, as the feed goes out.
        let newest = rig.status(&mut owner).await.last_event;
        let cursor = at_newest(&owner, newest);
        owner.subscribe(cursor, Samples::Off).await.unwrap();
        member.subscribe(cursor, Samples::Off).await.unwrap();
        let mut third = rig.owner(0xa2).await;
        third.subscribe(None, Samples::Off).await.unwrap();
        // New events keep coming meanwhile, so the two caught up ask after
        // each one, as a client following a busy feeder does.
        rig.fill_backlog(&mut owner).await;
        rig.feed(&mut owner).await;

        let owner_saw = until_fed(&mut owner, newest).await;
        let member_saw = until_fed(&mut member, newest).await;
        let third_saw = until_fed(&mut third, newest).await;
        assert_eq!(owner_saw, member_saw);
        assert_eq!(
            third_saw.first().map(|&(seq, _)| seq),
            Some(1),
            "from the oldest"
        );
        assert_eq!(after(&third_saw, newest), owner_saw, "and the same feed");
        let locks = third_saw
            .iter()
            .filter(|(_, body)| matches!(body, EventBody::Feeder(FeederEvent::Lock { .. })))
            .count();
        assert!(
            locks >= 2 * usize::from(BACKLOG),
            "the whole backlog: {locks}"
        );

        rig.sim.stop().await;
    });
}

#[test]
fn a_session_that_leaves_mid_read_stalls_nobody() {
    local(async {
        let mut rig = Rig::start().await;
        let mut owner = rig.owner(0xa1).await;
        rig.fill_backlog(&mut owner).await;
        let newest = rig.status(&mut owner).await.last_event;

        // One starts replaying the ring and leaves after its first event,
        // with the rest of its batch and its turn still held.
        let mut leaving = rig.owner(0xa2).await;
        leaving.subscribe(None, Samples::Off).await.unwrap();
        let Pushed::Event(first) = leaving.next(WAIT).await.unwrap() else {
            panic!("an event first");
        };
        assert_eq!(first.seq, 1);
        drop(leaving);

        let mut replaying = rig.owner(0xa3).await;
        replaying.subscribe(None, Samples::Off).await.unwrap();
        let cursor = at_newest(&owner, newest);
        owner.subscribe(cursor, Samples::Off).await.unwrap();
        rig.feed(&mut owner).await;

        let owner_saw = until_fed(&mut owner, newest).await;
        let replayed = until_fed(&mut replaying, newest).await;
        assert_eq!(replayed.first().map(|&(seq, _)| seq), Some(1));
        assert_eq!(after(&replayed, newest), owner_saw);

        rig.sim.stop().await;
    });
}
