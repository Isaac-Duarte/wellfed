//! How many failed logins the server takes: 5 a minute for a name from one
//! address, 30 a minute for a name from every address together, and 20 a
//! minute from one address across names, so a password cannot be guessed
//! at the rate the server hashes.
//!
//! A name is limited per address first, so someone guessing at a name from
//! their own address locks out only that address, never the account's
//! owner logging in from theirs. The name's ceiling across addresses bounds
//! guessing spread over many: six addresses' worth, far above an owner's
//! own retries, and 43,200 guesses a day at most, which a password of 10
//! characters or more outlasts. Six addresses are all it takes to hold the
//! ceiling down for as long as the flood goes on: six IPv4 addresses, or
//! six /64s, and one IPv6 /48 holds 65,536 of them.
//!
//! So an address the account logged in from within the last
//! [`KNOWN_FOR_MS`] skips the ceiling: it keeps its own bucket for the
//! name and its own for the address, and nothing from other addresses
//! spends either. Known addresses are the account's, by its id, not the
//! name's: names are compared as typed, so "Ada" and "ada" may be two
//! accounts, which the buckets' folding treats as one only to limit more. The
//! bar to lock an owner out is then to share their address (their home
//! network, their carrier's NAT), or to flood while they log in from
//! somewhere new: a new network, a /64 their carrier rotated, or after a
//! server restart, since the known addresses are kept in memory alone.
//! Only a login with the right password makes an address known, so
//! guessing never does, and at most [`KNOWN_PER_ACCOUNT`] addresses are
//! kept for an account and [`KNOWN`] in all, the oldest dropped first.
//!
//! Each of those is a token bucket, kept as the time it is next full (a
//! generic cell rate: one attempt spends a minute's share, and a bucket
//! refuses an attempt that would take it past a minute ahead). An attempt
//! spends from its three buckets before the password is checked, so a
//! flood of concurrent attempts cannot all pass a check made before any of
//! them failed; a login that succeeds gives its share back. No account is
//! locked: a limited attempt is refused `Retry-After` the time until each
//! of its buckets has a share, and sessions and tokens already open carry
//! on.
//!
//! The buckets live in one map of at most [`ENTRIES`], so a flood of
//! names or addresses costs bounded memory. A bucket is dropped only once
//! it is full again, when it holds nothing a later attempt could miss; the
//! map is kept in order of that time, so any such bucket is found at once.
//! While every bucket still holds state, an attempt that needs a new one
//! is refused until the first is full again: a flood of fresh names or
//! addresses can delay others' logins, never buy the name under attack
//! more guesses by pushing its bucket out. An IPv6 address counts by its
//! /64, the block a single host is usually given, so rotating within it
//! buys nothing.

use std::collections::{BTreeSet, HashMap};
use std::net::IpAddr;
use std::sync::Mutex;
use std::time::Duration;

use jiff::Timestamp;
use wellfed_api::UserId;

use crate::lock;

/// The most buckets kept.
pub(crate) const ENTRIES: usize = 10_000;

/// How long a login vouches for its name from its address: a month, long
/// enough for someone who logs in only now and then (a new browser, the
/// app after a reinstall, a session that ended) to stay known through a
/// flood, short enough that an address they left (an IPv4 address or a
/// /64 handed to someone else) soon stops counting.
const KNOWN_FOR_MS: i64 = 30 * 24 * 60 * MINUTE_MS;
/// The most addresses an account is known from: a household's networks
/// and carriers over a month, with room to spare. One account's holder
/// logging in from many addresses pushes out only that account's oldest.
const KNOWN_PER_ACCOUNT: usize = 16;
/// The most (account, address) pairs known in all.
const KNOWN: usize = 4_096;

/// Failed logins a name takes a minute from one address.
const PER_NAME_FROM: i64 = 5;
/// Failed logins a name takes a minute from every address together.
const PER_NAME: i64 = 30;
/// Failed logins an address takes a minute.
const PER_ADDRESS: i64 = 20;
const MINUTE_MS: i64 = 60_000;

/// The longest name a bucket keeps: longer than any account's, so it
/// limits the names that exist exactly, and bounds what a key holds.
const NAME_CHARS: usize = 64;

/// Whose bucket. A name is trimmed and lowercased, so its spellings share
/// one; an address is an IPv4 address, or an IPv6 address's /64.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum Key {
    /// A name from one address.
    NameFrom(String, IpAddr),
    /// A name from every address together: its ceiling.
    Name(String),
    /// An address, across names.
    Address(IpAddr),
}

impl Key {
    /// The buckets an attempt at `name` from `from` spends from: the name's
    /// ceiling too, unless the account is `Known` from that address.
    fn of(name: String, from: IpAddr, known: Known) -> Vec<Self> {
        let mut keys = vec![Self::NameFrom(name.clone(), from), Self::Address(from)];
        if known == Known::No {
            keys.push(Self::Name(name));
        }
        keys
    }

    fn spelling(name: &str) -> String {
        let name: String = name.trim().chars().take(NAME_CHARS).collect();
        name.to_lowercase()
    }

    fn block(address: IpAddr) -> IpAddr {
        match address.to_canonical() {
            IpAddr::V4(v4) => IpAddr::V4(v4),
            IpAddr::V6(v6) => {
                let prefix = u128::from(v6) & !u128::from(u64::MAX);
                IpAddr::V6(prefix.into())
            }
        }
    }

    /// The time an attempt spends, in milliseconds.
    const fn interval(&self) -> i64 {
        match self {
            Self::NameFrom(..) => MINUTE_MS / PER_NAME_FROM,
            Self::Name(_) => MINUTE_MS / PER_NAME,
            Self::Address(_) => MINUTE_MS / PER_ADDRESS,
        }
    }
}

/// A login refused for its rate: the next attempt is taken after
/// `retry_after`, a whole number of seconds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Limited {
    pub(crate) retry_after: Duration,
}

/// An attempt that spent from its buckets; [`Limits::succeeded`] gives the
/// spend back, and makes its address known for its account.
#[must_use = "a login that succeeds gives its attempt back"]
pub(crate) struct Attempt {
    account: Option<UserId>,
    from: IpAddr,
    keys: Vec<Key>,
}

/// Whether an account logged in from an address within [`KNOWN_FOR_MS`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Known {
    Yes,
    No,
}

#[derive(Debug, Default)]
pub(crate) struct Limits {
    buckets: Mutex<Buckets>,
    /// When each name last logged in from each address (Unix ms).
    known: Mutex<HashMap<(UserId, IpAddr), i64>>,
}

#[derive(Debug, Default)]
struct Buckets {
    /// Each bucket's next-full time (Unix ms).
    entries: HashMap<Key, i64>,
    /// The same, in order of that time, the first full again first.
    by_full: BTreeSet<(i64, Key)>,
    /// When a full map last warned that it refused a new bucket (Unix ms):
    /// a flood warns once a minute, not once an attempt.
    warned: Option<i64>,
}

impl Limits {
    /// Spend one attempt from the buckets of `name` from `from`, of `name`
    /// and of `from` at `now`, or refuse with the wait until all three take
    /// one, spending nothing. `account` is the account the name is, if it
    /// is one: one known from `from` skips the name's bucket.
    pub(crate) fn attempt(
        &self,
        name: &str,
        account: Option<UserId>,
        from: IpAddr,
        now: Timestamp,
    ) -> Result<Attempt, Limited> {
        let now = now.as_millisecond();
        let (name, from) = (Key::spelling(name), Key::block(from));
        let known = account.map_or(Known::No, |account| self.known(account, from, now));
        let keys = Key::of(name, from, known);
        let mut buckets = lock(&self.buckets);
        for key in &keys {
            buckets.rebase(key, now);
        }

        let wait = keys
            .iter()
            .map(|key| {
                let full = buckets.full(key, now);
                (full + key.interval() - now - MINUTE_MS).max(0)
            })
            .max()
            .unwrap_or(0);
        let new = keys
            .iter()
            .filter(|key| !buckets.entries.contains_key(key))
            .count();
        let room = buckets.make_room(&keys, new, now);
        if room > 0
            && buckets
                .warned
                .is_none_or(|warned| now - warned >= MINUTE_MS)
        {
            buckets.warned = Some(now);
            tracing::warn!(
                buckets = ENTRIES,
                "the login limits are full: logins from new names or addresses wait until a bucket refills"
            );
        }
        let wait = wait.max(room);
        if wait > 0 {
            // Whole seconds, rounded up, as `Retry-After` carries them.
            let retry_after = Duration::from_secs(wait.unsigned_abs().div_ceil(1000));
            return Err(Limited { retry_after });
        }

        for key in &keys {
            let full = buckets.full(key, now);
            buckets.set(key.clone(), full.max(now) + key.interval());
        }
        Ok(Attempt {
            account,
            from,
            keys,
        })
    }

    fn known(&self, account: UserId, from: IpAddr, now: i64) -> Known {
        let known = lock(&self.known);
        let since = known.get(&(account, from));
        if since.is_some_and(|&since| now - since < KNOWN_FOR_MS) {
            Known::Yes
        } else {
            Known::No
        }
    }

    /// Give back a login's attempt: it was no guess. A bucket full again
    /// holds nothing worth keeping, and is dropped. The account is known
    /// from the address from now on.
    pub(crate) fn succeeded(&self, attempt: Attempt, now: Timestamp) {
        let now = now.as_millisecond();
        if let Some(account) = attempt.account {
            self.vouch(account, attempt.from, now);
        }
        let mut buckets = lock(&self.buckets);
        for key in attempt.keys {
            let full = buckets.full(&key, now) - key.interval();
            if full <= now {
                buckets.remove(&key);
            } else {
                buckets.set(key, full);
            }
        }
    }

    /// Remember that `account` logged in from `from`, dropping what is past
    /// [`KNOWN_FOR_MS`], then the account's oldest address past
    /// [`KNOWN_PER_ACCOUNT`], then the oldest of all past [`KNOWN`]. A scan
    /// of a few thousand entries, and only on a login with the right
    /// password.
    fn vouch(&self, account: UserId, from: IpAddr, now: i64) {
        let mut known = lock(&self.known);
        known.retain(|_, since| now - *since < KNOWN_FOR_MS);
        known.remove(&(account, from));

        let oldest = |known: &HashMap<(UserId, IpAddr), i64>, of: Option<UserId>| {
            known
                .iter()
                .filter(|((whose, _), _)| of.is_none_or(|of| *whose == of))
                .min_by_key(|(_, since)| **since)
                .map(|(key, _)| *key)
        };
        let count = known.keys().filter(|(whose, _)| *whose == account).count();
        if count >= KNOWN_PER_ACCOUNT
            && let Some(key) = oldest(&known, Some(account))
        {
            known.remove(&key);
        }
        if known.len() >= KNOWN
            && let Some(key) = oldest(&known, None)
        {
            known.remove(&key);
        }
        known.insert((account, from), now);
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        lock(&self.buckets).entries.len()
    }

    #[cfg(test)]
    fn known_len(&self) -> usize {
        lock(&self.known).len()
    }
}

impl Buckets {
    /// When `key`'s bucket is next full, no later than a minute from now:
    /// a clock stepped back leaves no bucket refusing for longer than one
    /// full minute would.
    fn full(&self, key: &Key, now: i64) -> i64 {
        let full = self.entries.get(key).copied().unwrap_or(now);
        full.min(now + MINUTE_MS)
    }

    /// Bring `key`'s bucket back to at most a minute ahead of `now`, and
    /// keep it there: after the clock steps back, a refused attempt would
    /// otherwise leave the old deadline stored, and each later attempt
    /// would be refused anew until the clock caught up with it.
    fn rebase(&mut self, key: &Key, now: i64) {
        let latest = now + MINUTE_MS;
        if self.entries.get(key).is_some_and(|&full| full > latest) {
            self.set(key.clone(), latest);
        }
    }

    /// Drop buckets full again until `new` more fit, or say how long until
    /// the first that would have to go is full: a bucket still holding
    /// state is never dropped. The attempt's own `keys` are never the ones
    /// dropped, since it is about to set them, so the map never passes
    /// [`ENTRIES`].
    fn make_room(&mut self, keys: &[Key], new: usize, now: i64) -> i64 {
        let over = (self.entries.len() + new).saturating_sub(ENTRIES);
        let victims: Vec<(i64, Key)> = self
            .by_full
            .iter()
            .filter(|(_, key)| !keys.contains(key))
            .take(over)
            .cloned()
            .collect();
        let blocking = victims
            .iter()
            .map(|(full, _)| full.min(&(now + MINUTE_MS)) - now)
            .max()
            .unwrap_or(0);
        if blocking > 0 {
            return blocking;
        }

        for (_, key) in &victims {
            self.remove(key);
        }
        0
    }

    fn set(&mut self, key: Key, full: i64) {
        self.remove(&key);
        self.by_full.insert((full, key.clone()));
        self.entries.insert(key, full);
    }

    fn remove(&mut self, key: &Key) {
        if let Some(full) = self.entries.remove(key) {
            self.by_full.remove(&(full, key.clone()));
        }
    }
}

#[cfg(test)]
mod tests;
