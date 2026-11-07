//! The update path's words on the wire: what each state and refusal of
//! `wellfed_core::ota` becomes in the device API, so the firmware and
//! every host fake answer an owner the same way.

use wellfed_core::ota::install::{
    Bank0Check, Blocked, CandidateState, Phase, Progress, Refused, Stop, WANTED,
};
use wellfed_core::ota::offer::{KeyError, OfferError, StatementError};
use wellfed_core::store::update::UpdateError;
use wellfed_proto::api::{
    self, Bank0Blocked, FrozenRefusal, UpdateCandidate, UpdatePhase, UpdateRefusal, UpdateStop,
    UpdateView, WANTED_MAX,
};

const _: () = assert!(WANTED == WANTED_MAX);

pub const fn stop(stop: Stop) -> UpdateStop {
    match stop {
        Stop::Timeout => UpdateStop::Timeout,
        Stop::Cancelled => UpdateStop::Cancelled,
        Stop::Order => UpdateStop::Order,
        Stop::Marker => UpdateStop::Marker,
        Stop::Write => UpdateStop::Write,
        Stop::Hash => UpdateStop::Hash,
        Stop::Activate => UpdateStop::Activate,
        Stop::Store => UpdateStop::Store,
    }
}

/// What the update store held, as [`EventBody::UpdateAtBoot`] records it.
///
/// [`EventBody::UpdateAtBoot`]: wellfed_proto::api::EventBody::UpdateAtBoot
pub const fn candidate(state: CandidateState) -> UpdateCandidate {
    match state {
        CandidateState::None => UpdateCandidate::None,
        CandidateState::Pending => UpdateCandidate::Pending,
        CandidateState::Activated => UpdateCandidate::Activated,
    }
}

pub const fn blocked(blocked: Blocked) -> Bank0Blocked {
    match blocked {
        Blocked::Marker => Bank0Blocked::Marker,
        Blocked::Unrecorded => Bank0Blocked::Unrecorded,
        Blocked::Key => Bank0Blocked::Key,
        Blocked::Extent => Bank0Blocked::Extent,
        Blocked::Changed => Bank0Blocked::Changed,
        Blocked::Read => Bank0Blocked::Read,
    }
}

pub const fn bank0(check: Bank0Check) -> api::Bank0Check {
    match check {
        Bank0Check::Unchecked => api::Bank0Check::Unchecked,
        Bank0Check::Match => api::Bank0Check::Match,
        Bank0Check::Blocked(reason) => api::Bank0Check::Blocked {
            reason: blocked(reason),
        },
    }
}

pub const fn view(progress: &Progress) -> UpdateView {
    UpdateView {
        phase: match progress.phase {
            Phase::Idle => UpdatePhase::Idle,
            Phase::Checking => UpdatePhase::Checking,
            Phase::GivingWay => UpdatePhase::GivingWay,
            Phase::Receiving => UpdatePhase::Receiving,
            Phase::Activated => UpdatePhase::Activated,
        },
        bank: progress.bank,
        min_version: progress.min_version,
        digest: progress.digest,
        version: progress.version,
        image_len: progress.image_len,
        written: progress.written,
        wanted: progress.wanted,
        stopped: match progress.stopped {
            Some(s) => Some(stop(s)),
            None => None,
        },
        bank0: bank0(progress.bank0),
    }
}

const fn key(error: KeyError) -> UpdateRefusal {
    match error {
        KeyError::Unknown => UpdateRefusal::Key,
        KeyError::Revoked => UpdateRefusal::Revoked,
    }
}

const fn offer(error: OfferError) -> UpdateRefusal {
    match error {
        OfferError::Length | OfferError::Version | OfferError::Magic | OfferError::Kind => {
            UpdateRefusal::Manifest
        }
        OfferError::Key(error) => key(error),
        OfferError::Signature => UpdateRefusal::Signature,
        OfferError::Bank | OfferError::Size | OfferError::Model => UpdateRefusal::Target,
        OfferError::Same => UpdateRefusal::Same,
        OfferError::Downgrade => UpdateRefusal::Downgrade,
        OfferError::Statement(StatementError::Key(error)) => key(error),
        OfferError::Statement(_) => UpdateRefusal::Statement,
    }
}

const fn statement(error: StatementError) -> UpdateRefusal {
    match error {
        StatementError::Key(error) => key(error),
        StatementError::Signature => UpdateRefusal::Signature,
        StatementError::Length
        | StatementError::Magic
        | StatementError::Device
        | StatementError::Digest
        | StatementError::Floor => UpdateRefusal::Statement,
    }
}

/// Why an update request was refused, as the owner reads it: a frozen
/// refusal, since the update path is frozen.
pub const fn refusal(refused: Refused) -> FrozenRefusal {
    FrozenRefusal::Update(match refused {
        Refused::Busy => return FrozenRefusal::Busy,
        Refused::Store(UpdateError::Full | UpdateError::Occupied) => return FrozenRefusal::Full,
        Refused::Store(UpdateError::Revoked) => UpdateRefusal::Revoked,
        Refused::Store(_) | Refused::Unreadable => UpdateRefusal::Store,
        Refused::Offer(error) => offer(error),
        Refused::Statement(error) => statement(error),
        Refused::Fallback(reason) => UpdateRefusal::Fallback {
            reason: blocked(reason),
        },
        Refused::Pending => UpdateRefusal::Pending,
        Refused::Failed => UpdateRefusal::Failed,
        Refused::Attempt => UpdateRefusal::Attempt,
        Refused::Installing => UpdateRefusal::Installing,
        Refused::NotInstalling => UpdateRefusal::NotInstalling,
    })
}
