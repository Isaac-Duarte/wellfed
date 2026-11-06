//! An existing event stream must lose its original permissions after an account role changes.

use std::time::Duration;
use uuid::Uuid;
use wellfed_api::{
    AdoptFeeder, ClientRole, CommandQuery, Connection, Credentials, NewUser, Role, StreamEvent,
    StreamQuery, UserPatch,
};
use wellfed_server_client::Lock;
use wellfed_server_testkit::{TestServer, ada_setup, feeder, local, until};

#[test]
fn demoted_owner_must_not_keep_other_users_command_stream() {
    local(async {
        let sim = feeder::start().await;
        let server = TestServer::start().await;
        let mut ada = server.client();
        let ada_user = ada.setup(&ada_setup()).await.unwrap();
        let bob_user = ada
            .create_user(&NewUser {
                name: "review-bob".into(),
                password: "synthetic review password".into(),
                role: Role::Owner,
                expires: None,
            })
            .await
            .unwrap();
        let mut bob = server.client();
        bob.login(&Credentials {
            name: bob_user.name.clone(),
            password: "synthetic review password".into(),
        })
        .await
        .unwrap();
        let kitchen = ada
            .adopt(&AdoptFeeder {
                invite: feeder::invite(&sim, ClientRole::Server).await,
                name: "review-kitchen".into(),
                address: Some(sim.address.to_string()),
                phone_key: None,
            })
            .await
            .unwrap()
            .id;
        until("online", || async {
            ada.feeders().await.unwrap()[0].connection == Connection::Online
        })
        .await;
        let mut stream = bob.stream(&StreamQuery::default(), None).await.unwrap();
        ada.update_user(
            bob_user.id,
            &UserPatch {
                role: Some(Role::Member),
                ..UserPatch::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(bob.session().await.unwrap().role, Role::Member);
        println!("Bob demoted; wait beyond the 60-second stream reauthentication interval");
        tokio::time::sleep(Duration::from_secs(62)).await;
        let key = Uuid::from_u128(0x0192c5d0_9999_7000_8000_000000000001);
        ada.set_lock(kitchen, Lock::Unlocked, key).await.unwrap();
        let page = bob.commands(&CommandQuery::default()).await.unwrap();
        println!("Member HTTP command list: {page:?}");
        assert!(page.items.iter().all(|command| command.user != ada_user.id));
        let leaked = tokio::time::timeout(Duration::from_secs(3), async {
            while let Some(update) = stream.next().await {
                if let StreamEvent::Command { command } = update.unwrap().event
                    && command.user == ada_user.id
                {
                    return Some(command);
                }
            }
            None
        })
        .await
        .expect("the demoted owner's stream must close after reauthentication");
        println!("Other user's command delivered after demotion and 62 seconds: {leaked:?}");
        drop(stream);
        server.stop().await;
        sim.stop().await;
        assert!(
            leaked.is_none(),
            "PRIVILEGE NOT REVOKED: demoted owner still receives another user's command after periodic auth check"
        );
    });
}
