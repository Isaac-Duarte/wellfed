//! The released migrations, pinned. sqlx records each applied migration's
//! SHA-384 and refuses to open a database whose record differs from the
//! migration the build embeds, so editing a released migration (even a
//! comment) stops every server that ran it from starting. The fixture
//! lists each one; this test holds the embedded migrations to it.

use sqlx::migrate::Migrator;

static MIGRATOR: Migrator = sqlx::migrate!();

const PINNED: &str = include_str!("fixtures/migrations.txt");

struct Pinned<'a> {
    version: i64,
    checksum: &'a str,
    description: &'a str,
}

fn pinned() -> Vec<Pinned<'static>> {
    PINNED
        .lines()
        .filter(|line| !line.starts_with('#') && !line.trim().is_empty())
        .map(|line| {
            let mut fields = line.splitn(3, ' ');
            let (Some(version), Some(checksum), Some(description)) =
                (fields.next(), fields.next(), fields.next())
            else {
                panic!("a fixture line is `version sha384 description`: {line:?}");
            };
            Pinned {
                version: version.parse().unwrap(),
                checksum,
                description,
            }
        })
        .collect()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[test]
fn every_pinned_migration_is_embedded_unchanged() {
    for pin in pinned() {
        let Some(migration) = MIGRATOR.iter().find(|m| m.version == pin.version) else {
            panic!("migration {} is pinned but not embedded", pin.version);
        };

        assert_eq!(
            hex(&migration.checksum),
            pin.checksum,
            "migration {} changed: a server that ran it would refuse to start",
            pin.version
        );
        assert_eq!(migration.description, pin.description);
    }
}

#[test]
fn every_embedded_migration_is_pinned() {
    let pinned = pinned();

    for migration in MIGRATOR.iter() {
        assert!(
            pinned.iter().any(|pin| pin.version == migration.version),
            "append `{} {} {}` to tests/fixtures/migrations.txt",
            migration.version,
            hex(&migration.checksum),
            migration.description
        );
    }
}

#[test]
fn the_pins_are_in_version_order_with_no_repeats() {
    let versions: Vec<i64> = pinned().iter().map(|pin| pin.version).collect();

    assert!(
        versions.windows(2).all(|pair| pair[0] < pair[1]),
        "{versions:?}"
    );
}
