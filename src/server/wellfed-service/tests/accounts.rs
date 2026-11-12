//! Household accounts: the first owner's setup, login and logout, and the
//! limit on guessing a password through a password change.

use wellfed_api::{Credentials, PasswordChange, Role, Setup};
use wellfed_server_testkit::{HERE, TestService, ada_setup, local};
use wellfed_service::{AccountError, Error, Invalid};

#[test]
fn the_first_owner_sets_up_once_and_logs_in() {
    local(async {
        let server = TestService::start().await;
        let service = server.service();
        assert!(service.setup_open().await.unwrap());

        let weak = Setup {
            name: "ada".into(),
            password: "nine char".into(),
            zone: "UTC".into(),
        };
        assert!(matches!(
            service.setup(weak).await,
            Err(Error::Invalid(Invalid::Password))
        ));
        let setup = Setup {
            name: " ada ".into(),
            password: "correct horse".into(),
            zone: "UTC".into(),
        };
        let credentials = Credentials {
            name: "ada".into(),
            password: setup.password.clone(),
        };
        let (user, token) = service.setup(setup.clone()).await.unwrap();
        assert_eq!((user.name.as_str(), user.role), ("ada", Role::Owner));
        assert!(!service.setup_open().await.unwrap());
        assert!(matches!(
            service.setup(setup).await,
            Err(Error::Account(AccountError::SetupClosed))
        ));
        assert_eq!(
            service.authenticate(&token).await.unwrap(),
            Some(user.clone())
        );

        let wrong = Credentials {
            password: "wrong horse".into(),
            ..credentials.clone()
        };
        assert!(matches!(
            service.login(wrong, HERE).await,
            Err(Error::Account(AccountError::BadCredentials))
        ));
        let nobody = Credentials {
            name: "bob".into(),
            ..credentials.clone()
        };
        assert!(matches!(
            service.login(nobody, HERE).await,
            Err(Error::Account(AccountError::BadCredentials))
        ));
        let (again, second) = service.login(credentials, HERE).await.unwrap();
        assert_eq!(again, user);

        service.logout(&token).await.unwrap();
        assert_eq!(service.authenticate(&token).await.unwrap(), None);
        assert_eq!(service.authenticate(&second).await.unwrap(), Some(user));
        server.shutdown().await;
    });
}

/// Wrong current passwords count as failed logins: after the name's five a
/// minute the next is refused unhashed, so a stolen session cannot guess
/// the password at the rate the server hashes.
#[test]
fn wrong_current_passwords_are_limited_as_failed_logins_are() {
    local(async {
        let server = TestService::start().await;
        let service = server.service();
        let (ada, session) = service.setup(ada_setup()).await.unwrap();
        let guess = |n: u32| PasswordChange {
            current: format!("guess {n}"),
            password: "my own secret".into(),
        };

        for n in 0..5 {
            let wrong = service
                .change_password(&ada.clone().into(), Some(&session), guess(n), HERE)
                .await;
            assert!(
                matches!(wrong, Err(Error::Account(AccountError::BadCredentials))),
                "guess {n}: {wrong:?}"
            );
        }
        let limited = service
            .change_password(&ada.clone().into(), Some(&session), guess(5), HERE)
            .await;
        assert!(
            matches!(limited, Err(Error::Account(AccountError::Limited { .. }))),
            "{limited:?}"
        );
        server.shutdown().await;
    });
}
