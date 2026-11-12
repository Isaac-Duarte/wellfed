//! Which update a client may offer a feeder, and what the feeder shows once
//! an update's new build has restarted: the two rules every client that
//! updates a feeder shares (the app, the installer and through it the CLI),
//! so none restates them and none can disagree with another.
//!
//! **No client offers a feeder an older build.** A client sends no
//! downgrade statement, so an image at or below the version the feeder has
//! confirmed (its floor, `wellfed_model::UpdateView::min_version`) would be refused. A
//! feeder on a newer API revision ([`Skew::FeederAhead`]) is never offered
//! anything either: the client is the side to update. And while bank 1
//! refuses every update until bank 0 checks itself in, none is offered.
//!
//! **An update ends on what a fresh session reads after its restart.** The
//! bank comes from the update state and the build from the welcome, both
//! frozen, so a trial that fell back to bank 0, and a bank 1 that runs some
//! other build than the one activated, read as what they are.

use wellfed_model::{Bank0Blocked, Bank0Check, Skew};

/// What a client could offer: a checked image's build and version.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Candidate<'a> {
    pub build: &'a str,
    pub version: u32,
}

/// Whether to offer a [`Candidate`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Offer {
    /// Newer than what the feeder confirmed, and bank 0 allows it: offer
    /// it.
    Newer,
    /// The feeder runs this build already.
    Current,
    /// It is not newer than what the feeder confirmed, or the feeder runs a
    /// newer API revision than this client.
    Older,
    /// Bank 1 refuses every update until bank 0 checks itself in.
    Blocked(Bank0Blocked),
}

/// Whether to offer `candidate` to a feeder that runs `running` (its
/// status's build; `None` when this client cannot read it) at API `skew`, whose update state reports `confirmed`
/// (`wellfed_model::UpdateView::min_version`) and `bank0`.
pub fn offer(
    candidate: Candidate<'_>,
    running: Option<&str>,
    skew: Skew,
    confirmed: u32,
    bank0: Bank0Check,
) -> Offer {
    if running.is_some_and(|running| candidate.build.eq_ignore_ascii_case(running)) {
        return Offer::Current;
    }
    if skew == Skew::FeederAhead || candidate.version <= confirmed {
        return Offer::Older;
    }

    match bank0 {
        Bank0Check::Unchecked | Bank0Check::Match => Offer::Newer,
        Bank0Check::Blocked(reason) => Offer::Blocked(reason),
        // A state a newer build added: nothing says bank 0 may take an
        // update.
        Bank0Check::Unknown => Offer::Blocked(Bank0Blocked::Unknown),
    }
}

/// What a session opened after an update's restart reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Restarted<'a> {
    /// The build its welcome names.
    pub build: &'a str,
    /// The bank its update state answers from.
    pub bank: u8,
    /// The highest version it reports confirmed (`wellfed_model::UpdateView::min_version`).
    pub confirmed: u32,
    /// It reports itself ready to feed.
    pub ready: bool,
}

/// Where an activated update stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Confirmation {
    /// The new build runs and has not decided yet: look again.
    Pending,
    /// The new build runs and has confirmed itself.
    Confirmed,
    /// Bank 0 answers: the trial failed and the feeder gave way to its
    /// recovery copy, which may be older than what it ran before.
    FellBack,
    /// Bank 1 runs some other build than the one activated.
    WrongBuild,
}

/// Where the update that activated `served` at `version` stands, by what
/// `restarted` reads. Confirmed once bank 1 runs the build and either
/// reports the version confirmed, or can feed: a trial build takes no feed
/// until it confirms, so a ready one has confirmed. Readiness is what
/// settles a reinstall of a build the feeder confirmed before, which skips
/// the trial and leaves the floor below the version.
pub fn confirmation(served: &str, version: u32, restarted: Restarted<'_>) -> Confirmation {
    if restarted.bank == 0 {
        return Confirmation::FellBack;
    }
    if !restarted.build.eq_ignore_ascii_case(served) {
        return Confirmation::WrongBuild;
    }
    if restarted.confirmed >= version || restarted.ready {
        Confirmation::Confirmed
    } else {
        Confirmation::Pending
    }
}
