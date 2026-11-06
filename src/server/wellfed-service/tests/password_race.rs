//! A login already verifying a password must not outlive its rotation.

use std::sync::{
    Arc, Condvar, Mutex,
    atomic::{AtomicBool, Ordering},
};
use wellfed_api::{Credentials, PasswordChange, Setup};
use wellfed_server_testkit::fixed::{FixedClock, SeededRandom};
use wellfed_server_testkit::{HERE, TempStore, config};
use wellfed_service::{AccountError, Caller, Error, Random, RandomFailed, Service};

struct GatedRandom {
    random: SeededRandom,
    armed: AtomicBool,
    arrived: tokio::sync::Notify,
    released: (Mutex<bool>, Condvar),
}

impl Random for GatedRandom {
    fn fill(&self, out: &mut [u8]) -> Result<(), RandomFailed> {
        if out.len() == 32 && self.armed.swap(false, Ordering::SeqCst) {
            self.arrived.notify_one();
            let (lock, cv) = &self.released;
            let released = lock.lock().unwrap();
            let (released, wait) = cv
                .wait_timeout_while(released, std::time::Duration::from_secs(15), |released| {
                    !*released
                })
                .unwrap();
            assert!(
                *released && !wait.timed_out(),
                "the password rotation releases the login"
            );
        }
        self.random.fill(out)
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_login_verified_before_rotation_cannot_create_a_session_after_it() {
    let store = TempStore::new();
    let mut cfg = config();
    cfg.releases.dir = store.releases();
    cfg.releases.stock.dir = store.stock();
    cfg.backups.dir = store.backups();
    let random = Arc::new(GatedRandom {
        random: SeededRandom::new(102),
        armed: AtomicBool::new(false),
        arrived: tokio::sync::Notify::new(),
        released: (Mutex::new(false), Condvar::new()),
    });
    let service = Service::start(
        store.open().await,
        cfg,
        Arc::new(FixedClock),
        random.clone(),
    )
    .await
    .unwrap();
    let (user, owner_session) = service
        .setup(Setup {
            name: "owner".into(),
            password: "old review password".into(),
            zone: "UTC".into(),
        })
        .await
        .unwrap();
    let caller = Caller::from(user.clone());
    random.armed.store(true, Ordering::SeqCst);
    let login_service = service.clone();
    let old_login = tokio::spawn(async move {
        login_service
            .login(
                Credentials {
                    name: "owner".into(),
                    password: "old review password".into(),
                },
                HERE,
            )
            .await
    });
    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        random.arrived.notified(),
    )
    .await
    .unwrap();
    let rotation = service
        .change_password(
            &caller,
            Some(&owner_session),
            PasswordChange {
                current: "old review password".into(),
                password: "new review password".into(),
            },
        )
        .await;
    *random.released.0.lock().unwrap() = true;
    random.released.1.notify_one();
    rotation.unwrap();
    assert!(matches!(
        old_login.await.unwrap(),
        Err(Error::Account(AccountError::BadCredentials))
    ));
    assert_eq!(
        service.authenticate(&owner_session).await.unwrap(),
        Some(user)
    );
    let (_, new_session) = service
        .login(
            Credentials {
                name: "owner".into(),
                password: "new review password".into(),
            },
            HERE,
        )
        .await
        .unwrap();
    assert!(service.authenticate(&new_session).await.unwrap().is_some());
    service.shutdown().await;
}
