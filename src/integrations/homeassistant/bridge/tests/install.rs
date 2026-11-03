//! Firmware installs from Home Assistant against the simulator's update
//! path, the firmware's own update stage on two simulated banks: a release
//! is verified into the cache, then pushed over the bridge's one session,
//! which the feeder's restart into bank 0 drops and the session thread
//! reopens on its own.

use std::sync::atomic::{AtomicU8, Ordering};
use std::time::{Duration, Instant};

use wellfed_client::api::Invite;
use wellfed_connect::Connection;
use wellfed_feeder_sim::harness::Threaded;
use wellfed_homeassistant::api::{Installed, NotSent, Signer, UpdateKind};
use wellfed_homeassistant::feeder::{Feeder, Target};
use wellfed_homeassistant::key::{ClientKey, join};
use wellfed_homeassistant::releases::{Library, block_on};
use wellfed_model::{Answer, ClientRole, Op};
use wellfed_server_testkit::release::{self, SECTOR};
use wellfed_session::hex::Lower;
use wellfed_session::noise::Keypair;

const OWNER: [u8; 32] = [0x21; 32];
const WAIT: Duration = Duration::from_secs(120);
const BUILD: [u8; 32] = [0x77; 32];
const LEN: usize = 4 * SECTOR;

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

/// `op` from the simulator's owner, under a request id of its own.
fn owner(sim: &Threaded, op: &Op) -> Answer {
    static IDS: AtomicU8 = AtomicU8::new(1);
    let id = [IDS.fetch_add(1, Ordering::Relaxed); 16];
    runtime().block_on(async {
        let owner = Keypair::from_secret(OWNER);
        let mut connection = Connection::open(&owner, sim.identity, sim.address, [id[0]; 32])
            .await
            .unwrap();
        connection.ask(id, op).await.unwrap()
    })
}

/// A simulator trusting the project's test key, and the integration's key
/// joined to it as `role`.
fn joined(role: ClientRole) -> (Threaded, ClientKey) {
    let project = release::project().verifying_key().to_bytes();
    let sim = Threaded::start_updatable(*Keypair::from_secret(OWNER).public(), project);
    let Answer::Invite(invite) = owner(&sim, &Op::CreateInvite { role }) else {
        panic!("no invite");
    };
    let text = Invite {
        feeder: sim.identity,
        id: invite.id,
        secret: invite.secret.0,
    }
    .text();

    let key = ClientKey::generate().unwrap();
    let address = sim.address.to_string();
    let joined = runtime()
        .block_on(join(&key, &text, "home-assistant", Some(&address)))
        .unwrap();
    assert_eq!(joined, role);
    (sim, key)
}

fn session(sim: &Threaded, key: ClientKey) -> Feeder {
    let target = Target {
        identity: sim.identity,
        address: Some(sim.address.to_string()),
    };
    let feeder = Feeder::start(key, target).unwrap();
    let deadline = Instant::now() + WAIT;
    while Instant::now() < deadline {
        if feeder
            .next_update(Duration::from_secs(1))
            .is_some_and(|update| update.state.is_some())
        {
            return feeder;
        }
    }
    panic!("no state within {WAIT:?}");
}

/// A cache in a fresh folder, trusting the project's test key.
fn library(folder: &tempfile::TempDir) -> Library {
    let project = release::project().verifying_key().to_bytes().to_vec();
    Library::new(folder.path().to_path_buf(), vec![project], Vec::new()).unwrap()
}

#[test]
fn an_owner_installs_a_verified_release_over_the_one_session() {
    let (sim, key) = joined(ClientRole::Owner);
    let feeder = std::sync::Arc::new(session(&sim, key));
    let folder = tempfile::tempdir().unwrap();
    let library = library(&folder);

    let file = release::signed(&release::project(), 7, BUILD, LEN);
    let offer = block_on(library.add(file)).unwrap().unwrap();
    assert_eq!((offer.version, offer.signer), (7, Signer::Project));
    assert_eq!(offer.build, Lower(&BUILD).to_string());
    let newest = block_on(library.newest(false)).unwrap().unwrap();
    assert_eq!(newest, Some(offer.clone()));

    let installing = {
        let feeder = std::sync::Arc::clone(&feeder);
        let id = wellfed_homeassistant::releases::release_id(&offer.id).unwrap();
        std::thread::spawn(move || feeder.install_release(&library, id))
    };
    let mut written = 0;
    let deadline = Instant::now() + WAIT;
    while !installing.is_finished() {
        assert!(
            Instant::now() < deadline,
            "the install ends within {WAIT:?}"
        );
        if let Some(update) = feeder.next_update(Duration::from_millis(100))
            && update.kind == UpdateKind::Installing
        {
            written = written.max(update.install.unwrap().written);
        }
    }
    let installed = installing.join().unwrap().unwrap();
    assert_eq!(installed.result, Installed::Activated, "{installed:?}");
    assert!(written > 0, "progress came as updates");

    // The feeder restarts into the release, and the session follows it.
    let build = Lower(&BUILD).to_string();
    let deadline = Instant::now() + WAIT;
    loop {
        assert!(Instant::now() < deadline, "the session reaches the release");
        let state = feeder
            .next_update(Duration::from_secs(1))
            .and_then(|update| update.state);
        if let Some(state) = state
            && state.status.bank == 1
            && state.status.build.eq_ignore_ascii_case(&build)
        {
            break;
        }
    }
}

#[test]
fn a_member_s_install_is_never_sent_and_a_stranger_s_release_never_kept() {
    let (sim, key) = joined(ClientRole::Member);
    let feeder = session(&sim, key);
    let folder = tempfile::tempdir().unwrap();
    let library = library(&folder);

    let stranger = ed25519_dalek::SigningKey::from_bytes(&[0x66; 32]);
    let refused = block_on(library.add(release::signed(&stranger, 7, BUILD, LEN))).unwrap();
    assert!(refused.is_err(), "a release signed by an unknown key");

    let file = release::signed(&release::project(), 7, BUILD, LEN);
    let installed = feeder.install(file, BUILD);
    assert_eq!(installed.result, Installed::NotSent);
    assert_eq!(installed.not_sent, Some(NotSent::Forbidden));
}
