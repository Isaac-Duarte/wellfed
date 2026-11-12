//! A claim that puts a key a logout left with no token under a token must
//! prove the phone holds the key's secret, so another token of the same
//! account (a stolen one) cannot take the logged-out phone's key and then
//! have it revoked with itself. The proof answers a challenge the claiming
//! token asked for, once and before it expires, with a MAC only the key's
//! secret makes.

use std::sync::Arc;
use std::time::Duration;

use wellfed_api::{
    ClientOwner, ClientRole, Connection as Link, FeederId, Hex, KeyProof, LogOut, NewPhoneInvite,
    NewToken, PhoneChallenge, PhoneClaim, PhoneKey, User,
};
use wellfed_server_testkit::feeder::{self, Running};
use wellfed_server_testkit::fixed::ManualClock;
use wellfed_server_testkit::{TestService, ada_setup, local};
use wellfed_service::{Caller, ClientError, Error, Result, Service};
use wellfed_session::noise::Keypair;
use wellfed_session::proof;

const PHONE: u8 = 40;
const THIEF: u8 = 41;

fn secret(n: u8) -> [u8; 32] {
    [n; 32]
}

fn key(n: u8) -> [u8; 32] {
    feeder::phone_key(secret(n))
}

async fn token_of(service: &Service, user: &User) -> Caller {
    let login = Caller::from(user.clone());
    let new = NewToken {
        name: "phone".into(),
        expires: None,
    };
    let token = service.create_token(&login, new).await.unwrap();
    Caller {
        user: login.user,
        token: Some(token.token.id),
    }
}

/// The owner's phone `PHONE` joined the feeder and logged out, its key the
/// account's with no token, and another token of the same account.
struct LoggedOut {
    server: TestService,
    sim: Running,
    clock: ManualClock,
    id: FeederId,
    owner: User,
    other: Caller,
}

impl LoggedOut {
    async fn start() -> Self {
        let sim = feeder::start().await;
        let clock = ManualClock::new();
        let server = TestService::start_with_clock(Arc::new(clock.clone())).await;
        let id = server
            .adopt_as(&sim, &sim.address.to_string(), ClientRole::Owner)
            .await;
        server.link(id, Link::Online).await;
        let service = server.service();
        let owner = service.setup(ada_setup()).await.unwrap().0;

        let phone = token_of(service, &owner).await;
        let invite = NewPhoneInvite {
            key: Hex(key(PHONE)),
        };
        let invite = service.invite_phone(&phone, id, invite).await.unwrap();
        feeder::join_phone(&sim, &invite.invite, secret(PHONE)).await;
        let login = Caller::from(owner.clone());
        let own = LogOut {
            keys: vec![Hex(key(PHONE))],
        };
        service
            .log_out_token(&login, phone.token.unwrap(), &own)
            .await
            .unwrap();

        let other = token_of(service, &owner).await;
        Self {
            server,
            sim,
            clock,
            id,
            owner,
            other,
        }
    }

    fn service(&self) -> &Service {
        self.server.service()
    }

    async fn claim(&self, caller: &Caller, proof: Option<KeyProof>) -> Result<PhoneKey> {
        let claim = PhoneClaim {
            key: Hex(key(PHONE)),
            proof,
        };
        self.service().claim_phone(caller, self.id, claim).await
    }

    async fn challenge(&self, caller: &Caller) -> PhoneChallenge {
        self.service().phone_challenge(caller).await.unwrap()
    }

    /// The phone's key is still the account's with no token.
    async fn still_tokenless(&self) {
        let clients = self.service().clients(self.id).await.unwrap();
        let owner = clients
            .iter()
            .find(|client| client.key.0 == key(PHONE))
            .map(|client| &client.owner);
        assert!(
            matches!(owner, Some(ClientOwner::Phone { user, token: None, revoking: None }) if *user == self.owner.id),
            "{owner:?}"
        );
    }

    async fn stop(self) {
        self.server.shutdown().await;
        self.sim.stop().await;
    }
}

/// What phone `n`'s secret answers `challenge` with, bound to `caller`'s
/// token.
fn answer(challenge: &PhoneChallenge, caller: &Caller, n: u8) -> KeyProof {
    let holder = Keypair::from_secret(secret(n));
    let token = caller.token.unwrap().0;
    let mac = proof::prove(
        &holder,
        &challenge.key.0,
        &challenge.nonce.0,
        token.as_bytes(),
    );
    KeyProof {
        challenge: challenge.id,
        mac: Hex(mac.unwrap()),
    }
}

#[track_caller]
fn refused(result: Result<PhoneKey>, want: ClientError) {
    assert!(
        matches!(&result, Err(Error::Client(error)) if *error == want),
        "{result:?}"
    );
}

/// Another token of the account claims the logged-out phone's key: with
/// no proof, and with a proof its own key made, it is refused and the key
/// stays with no token.
#[test]
fn another_token_cannot_take_a_logged_out_key_without_its_secret() {
    local(async {
        let setup = LoggedOut::start().await;
        let thief = &setup.other;

        refused(
            setup.claim(thief, None).await,
            ClientError::PhoneKeyProofRequired,
        );
        let challenge = setup.challenge(thief).await;
        let forged = answer(&challenge, thief, THIEF);
        refused(
            setup.claim(thief, Some(forged)).await,
            ClientError::PhoneKeyProofRefused,
        );
        setup.still_tokenless().await;
        setup.stop().await;
    });
}

/// A proof answers a challenge of the claiming token's, once, before it
/// expires: another token's challenge, a used one and an expired one are
/// refused, and a token holds a bounded number, its oldest dropped.
#[test]
fn a_proof_answers_its_own_tokens_challenge_once_before_it_expires() {
    local(async {
        let setup = LoggedOut::start().await;
        let phone = token_of(setup.service(), &setup.owner).await;

        let theirs = setup.challenge(&setup.other).await;
        let proof = answer(&theirs, &phone, PHONE);
        refused(
            setup.claim(&phone, Some(proof)).await,
            ClientError::PhoneKeyProofRefused,
        );

        let expiring = setup.challenge(&phone).await;
        setup.clock.advance(Duration::from_secs(61));
        let late = answer(&expiring, &phone, PHONE);
        refused(
            setup.claim(&phone, Some(late)).await,
            ClientError::PhoneKeyProofRefused,
        );

        let first = setup.challenge(&phone).await;
        for _ in 0..8 {
            setup.challenge(&phone).await;
        }
        let dropped = answer(&first, &phone, PHONE);
        refused(
            setup.claim(&phone, Some(dropped)).await,
            ClientError::PhoneKeyProofRefused,
        );
        setup.still_tokenless().await;

        let challenge = setup.challenge(&phone).await;
        let proof = answer(&challenge, &phone, PHONE);
        let claimed = setup.claim(&phone, Some(proof)).await.unwrap();
        assert_eq!(claimed.token, phone.token);
        refused(
            setup.claim(&setup.other, Some(proof)).await,
            ClientError::PhoneKeyProofRefused,
        );
        setup.stop().await;
    });
}

/// The phone that holds the key proves it under its new token, so revoking
/// that token revokes the key.
#[test]
fn the_phone_proves_its_key_under_its_new_token_and_goes_with_it() {
    local(async {
        let setup = LoggedOut::start().await;
        let phone = token_of(setup.service(), &setup.owner).await;

        let challenge = setup.challenge(&phone).await;
        let proof = answer(&challenge, &phone, PHONE);
        let claimed = setup.claim(&phone, Some(proof)).await.unwrap();
        assert_eq!(claimed.token, phone.token);
        setup
            .service()
            .revoke_token(&phone, phone.token.unwrap())
            .await
            .unwrap();
        wellfed_server_testkit::until("the phone's key is revoked", || async {
            !feeder::client_keys(&setup.sim).await.contains(&key(PHONE))
        })
        .await;
        setup.stop().await;
    });
}
