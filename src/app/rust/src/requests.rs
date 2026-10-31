//! A request from Dart as the device API's ops, and the answer as Dart's
//! types. Each op goes through the session's loop under a fresh id and never
//! twice; only an idempotent chunk the feeder said it was too busy for is
//! sent again, and an unanswered one ends the upload.

use std::net::Ipv4Addr;
use std::str::FromStr;
use std::time::Instant;

use jiff::Timestamp;
use jiff::tz::TimeZone;
use tokio::sync::{mpsc, oneshot};
use wellfed_client::api::Invite;
use wellfed_client::call::{Clip, Step, Upload, UploadError, UploadFailed};
use wellfed_client::plans::{self, EditError};
use wellfed_client::setup::Credentials;
use wellfed_client::trace::Pages;
use wellfed_model::{
    Answer, Calibration, CalibrationView, CallView, Client, ClientRole, Generation, NtpServer, Op,
    Plan, Plans, Refusal, Settings, SettingsPatch, TimeChanges, TimeOfDay, TimeView, TraceEntry,
    TraceMotor, Weekday, Weekdays,
};

use crate::api::feeder::{
    CalibrationInfo, CallInfo, ClientView, FeederRefusal, FeederReply, FeederRequest, InviteView,
    NotAsked, NtpView, PlanEdit, PlanView, PlansView, PortionChange, SettingsView, TimeInfo,
    TraceView, UpdateInfo, ZoneChange,
};
use crate::session::Command;

/// How a request's one op ended, short of an answer.
enum Lost {
    Unanswered,
    NotSent,
}

impl From<Lost> for FeederReply {
    fn from(lost: Lost) -> Self {
        match lost {
            Lost::Unanswered => Self::Unanswered,
            Lost::NotSent => Self::NotSent,
        }
    }
}

/// One op through the session's loop. A session that ended before sending it
/// drops the reply; one that sent it answers `None` if it heard nothing.
async fn ask(commands: &mpsc::UnboundedSender<Command>, op: Op) -> Result<Answer, Lost> {
    let (reply, answer) = oneshot::channel();
    commands
        .send(Command::Ask { op, reply })
        .map_err(|_| Lost::NotSent)?;
    answer
        .await
        .map_err(|_| Lost::NotSent)?
        .ok_or(Lost::Unanswered)
}

/// Run `request` over the session behind `commands`, with feeder `identity`.
pub async fn run(
    commands: &mpsc::UnboundedSender<Command>,
    identity: [u8; 32],
    request: FeederRequest,
) -> FeederReply {
    let op = match request {
        FeederRequest::GetSettings => Op::GetSettings,
        FeederRequest::PutSettings { base, changes } => Op::PutSettings {
            base: Generation(base),
            changes: SettingsPatch {
                lights: changes.lights,
                sound: changes.sound,
                volume: changes.volume,
                lights_window: changes.lights_window,
                sound_window: changes.sound_window,
                desiccant_days: changes.desiccant_days,
                meal_reminder: changes.meal_reminder,
            },
        },
        FeederRequest::PutWifi {
            base,
            ssid,
            passphrase,
        } => match Credentials::new(ssid.into_bytes(), passphrase) {
            Ok(credentials) => Op::PutWifi {
                base: Generation(base),
                wifi: Some(credentials.into_wifi()),
            },
            Err(_) => return not_asked(NotAsked::Credentials),
        },
        FeederRequest::ForgetWifi { base } => Op::PutWifi {
            base: Generation(base),
            wifi: None,
        },
        FeederRequest::SetLock { locked } => Op::SetLock { locked },
        FeederRequest::Play { tune } => Op::Play { tune },
        FeederRequest::SyncClock => Op::TimeSync(wellfed_client::time::sync(
            Timestamp::now(),
            &TimeZone::system(),
        )),
        FeederRequest::GetTime => Op::GetTime,
        FeederRequest::PutTime { base, zone, ntp } => match time_changes(zone, ntp) {
            Ok(changes) => Op::PutTime {
                base: Generation(base),
                changes,
            },
            Err(reason) => return not_asked(reason),
        },
        FeederRequest::CreateInvite { role } => {
            return match ask(commands, Op::CreateInvite { role }).await {
                Ok(Answer::Invite(invite)) => FeederReply::Invite {
                    invite: invite_of(invite, identity, role),
                },
                Ok(answer) => reply_of(answer),
                Err(lost) => lost.into(),
            };
        }
        FeederRequest::ListClients => Op::ListClients,
        FeederRequest::RevokeClient { key } => match <[u8; 32]>::try_from(key) {
            Ok(key) => Op::RevokeClient { key },
            Err(_) => return not_asked(NotAsked::BadKey),
        },
        FeederRequest::Reboot => Op::Reboot,
        FeederRequest::ResetDesiccant => Op::ResetDesiccant,
        FeederRequest::Stats { section } => Op::Stats { section },
        FeederRequest::ReadTrace { feed } => return read_trace(commands, feed).await,
        FeederRequest::GetCalibration => Op::GetCalibration,
        FeederRequest::PutCalibration {
            base,
            portion,
            bowl,
        } => match portion_mg(portion) {
            Ok(portion_mg) => Op::PutCalibration {
                base: Generation(base),
                calibration: Calibration { portion_mg, bowl },
            },
            Err(reason) => return not_asked(reason),
        },
        FeederRequest::GetPlans => Op::GetPlans,
        FeederRequest::EditPlans { plans, edit } => {
            let held = plans_of(&plans);
            match edit_of(edit).and_then(|edit| plans::edited(&held, edit).map_err(not_edited)) {
                Ok(plans) => Op::PutPlans {
                    base: held.generation,
                    plans,
                },
                Err(reason) => return not_asked(reason),
            }
        }
        FeederRequest::GetCall => Op::GetCall,
        FeederRequest::UploadCall { wav } => return upload_call(commands, &wav).await,
        FeederRequest::DeleteCall => Op::DeleteCall,
        FeederRequest::UpdateState => Op::UpdateState,
        FeederRequest::CancelUpdate => Op::CancelUpdate,
        FeederRequest::TailLog { after } => Op::TailLog { after },
    };

    match ask(commands, op).await {
        Ok(answer) => reply_of(answer),
        Err(lost) => lost.into(),
    }
}

const fn not_asked(reason: NotAsked) -> FeederReply {
    FeederReply::NotAsked { reason }
}

/// The feed's trace, a page at a time. Reads run again freely, but one with
/// no answer ends the read: the session is going away.
async fn read_trace(commands: &mpsc::UnboundedSender<Command>, feed: u32) -> FeederReply {
    let mut pages = Pages::new(feed);
    loop {
        let answer = match ask(commands, pages.next_op()).await {
            Ok(answer) => answer,
            Err(lost) => return lost.into(),
        };
        if let Answer::Refused(refusal) = answer {
            return FeederReply::Refused {
                refusal: refusal_of(refusal),
            };
        }
        match pages.take(answer) {
            Ok(Some(trace)) => {
                return FeederReply::Trace {
                    trace: TraceView {
                        summary: trace.summary,
                        stored: trace.stored,
                        portion_ms: portion_times(&trace.entries, trace.summary.completed),
                        entries: trace.entries,
                    },
                };
            }
            Ok(None) => {}
            Err(_) => return FeederReply::Unexpected,
        }
    }
}

async fn upload_call(commands: &mpsc::UnboundedSender<Command>, wav: &[u8]) -> FeederReply {
    let Ok(clip) = Clip::from_wav(wav) else {
        return not_asked(NotAsked::BadRecording);
    };
    let mut upload = Upload::new(&clip);
    let mut started = false;
    loop {
        match upload.poll(Instant::now()) {
            Step::Ask(op) => match ask(commands, op).await {
                Ok(answer) => {
                    started = true;
                    upload.answer(&answer, Instant::now());
                }
                // Part of it went out, and the feeder's old clip with it, so
                // "not sent" would be untrue. Starting over is the owner's call.
                Err(Lost::NotSent) if started => return FeederReply::Unanswered,
                Err(lost) => return lost.into(),
            },
            Step::Wait(wait) => tokio::time::sleep(wait).await,
            Step::Stored(view) => {
                return FeederReply::Call {
                    call: call_of(view),
                };
            }
            Step::Failed(UploadFailed::Upload(UploadError::Refused(refusal))) => {
                return FeederReply::Refused {
                    refusal: refusal_of(refusal),
                };
            }
            Step::Failed(_) => return FeederReply::Unexpected,
        }
    }
}

fn time_changes(zone: Option<ZoneChange>, ntp: Option<String>) -> Result<TimeChanges, NotAsked> {
    let now = Timestamp::now();
    let named = |name: &str| {
        let host = TimeZone::get(name).ok();
        wellfed_client::time::rule(name, now, host.as_ref()).map_err(|_| NotAsked::BadZone)
    };
    let zone = match zone {
        None => None,
        Some(ZoneChange::Clear) => Some(String::new()),
        Some(ZoneChange::Named { name }) => Some(named(&name)?),
        Some(ZoneChange::Phone) => {
            let system = TimeZone::system();
            Some(named(system.iana_name().ok_or(NotAsked::BadZone)?)?)
        }
    };
    let ntp = ntp
        .map(|text| NtpServer::from_str(&text).map_err(|_| NotAsked::BadNtp))
        .transpose()?;
    Ok(TimeChanges { zone, ntp })
}

fn portion_mg(portion: PortionChange) -> Result<Option<u32>, NotAsked> {
    use wellfed_client::calibration::{mean_mg, parse_grams};

    match portion {
        PortionChange::Clear => Ok(None),
        PortionChange::Milligrams { mg } => Ok(Some(mg)),
        PortionChange::Weighed { grams, portions } => {
            let weighed = parse_grams(&grams).map_err(|_| NotAsked::BadWeight)?;
            let mean = mean_mg(weighed, portions).map_err(|_| NotAsked::BadWeight)?;
            Ok(Some(mean))
        }
    }
}

const fn not_edited(error: EditError) -> NotAsked {
    match error {
        EditError::Empty { .. } => NotAsked::EmptySlot,
        EditError::Full => NotAsked::PlansFull,
        EditError::Nothing => NotAsked::NothingToChange,
        EditError::Invalid => NotAsked::BadPlan,
    }
}

fn edit_of(edit: PlanEdit) -> Result<plans::Edit, NotAsked> {
    Ok(match edit {
        PlanEdit::Add { plan } => plans::Edit::Add(plan_of(&plan)),
        PlanEdit::Change {
            slot,
            minute,
            portions,
            days,
            call,
            enabled,
        } => plans::Edit::Change {
            slot,
            at: minute.map(TimeOfDay),
            portions,
            days: days.as_deref().map(weekdays_of),
            call,
            enabled,
        },
        PlanEdit::Enabled { slot, enabled } => plans::Edit::Enabled { slot, enabled },
        PlanEdit::Delete { slot } => plans::Edit::Delete { slot },
    })
}

fn weekdays_of(days: &[Weekday]) -> Weekdays {
    let bits = Weekday::ALL
        .iter()
        .enumerate()
        .filter(|(_, day)| days.contains(day))
        .fold(0u8, |bits, (bit, _)| bits | (1 << bit));
    Weekdays::from_bits(bits)
}

pub(crate) fn days_of(days: Weekdays) -> Vec<Weekday> {
    Weekday::ALL
        .iter()
        .enumerate()
        .filter(|(bit, _)| days.bits() & (1 << bit) != 0)
        .map(|(_, day)| *day)
        .collect()
}

fn plan_of(plan: &PlanView) -> Plan {
    Plan {
        minute: plan.minute,
        portions: plan.portions,
        days: weekdays_of(&plan.days),
        enabled: plan.enabled,
        call: plan.call,
    }
}

fn plans_of(view: &PlansView) -> Plans {
    let mut plans = [None; wellfed_model::PLANS];
    for (slot, plan) in plans.iter_mut().zip(&view.plans) {
        *slot = plan.as_ref().map(plan_of);
    }
    Plans {
        generation: Generation(view.generation),
        writable: view.writable,
        plans,
    }
}

pub(crate) fn plans_view(plans: &Plans) -> PlansView {
    PlansView {
        generation: plans.generation.0,
        writable: plans.writable,
        plans: plans
            .plans
            .iter()
            .map(|plan| {
                plan.map(|plan| PlanView {
                    minute: plan.minute,
                    portions: plan.portions,
                    days: days_of(plan.days),
                    enabled: plan.enabled,
                    call: plan.call,
                })
            })
            .collect(),
    }
}

pub(crate) fn settings_view(settings: &Settings) -> SettingsView {
    SettingsView {
        generation: settings.generation.0,
        lights: settings.lights,
        sound: settings.sound,
        volume: settings.volume,
        lights_window: settings.lights_window,
        sound_window: settings.sound_window,
        desiccant_days: settings.desiccant_days,
        desiccant_replaced: settings.desiccant_replaced.map(|date| date.to_string()),
        meal_reminder: settings.meal_reminder,
        writable: settings.writable,
    }
}

/// The feeder's time as read, and this phone's clocks when it was. The view's
/// numbers are all as of that moment, while the card is built again long after
/// it, so each stands against the read and never against the build.
pub(crate) struct TimeRead {
    pub(crate) view: TimeView,
    /// Places "last set" in the phone's own time.
    read_at: Timestamp,
    /// Measures what has passed since, so a step of the phone's clock does not
    /// move the feeder's. The session reads the time again about once a minute,
    /// which bounds what this drifts by.
    read_mono: Instant,
}

impl TimeRead {
    pub(crate) const fn new(view: TimeView, read_at: Timestamp, read_mono: Instant) -> Self {
        Self {
            view,
            read_at,
            read_mono,
        }
    }

    /// A view that just came in.
    pub(crate) fn just_read(view: TimeView) -> Self {
        Self::new(view, Timestamp::now(), Instant::now())
    }

    /// The read as of `now`. The clock runs on, and its error grows at the
    /// view's rate bound, the feeder's own reckoning between fixes. When it
    /// was last set does not move.
    pub(crate) fn info(&self, now: Instant) -> TimeInfo {
        let view = &self.view;
        let ntp = match view.ntp {
            NtpServer::Off => NtpView::Off,
            NtpServer::Gateway { port } => NtpView::Gateway { port },
            NtpServer::Address { ip, port } => NtpView::Address {
                ip: Ipv4Addr::to_string(&ip),
                port,
            },
        };
        let read_ms = self.read_at.as_millisecond();
        let elapsed = now.saturating_duration_since(self.read_mono);
        let elapsed_ms = i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX);

        TimeInfo {
            generation: view.generation.0,
            writable: view.writable,
            zone: view.zone.clone(),
            ntp,
            clock_ms: view
                .clock_ms
                .and_then(|ms| i64::try_from(ms).ok())
                .map(|ms| ms.saturating_add(elapsed_ms)),
            error_ms: view
                .error_ms
                .map(|error| error.saturating_add(grown_ms(elapsed_ms, view.rate_bound_ppb))),
            offset_s: view.offset_s,
            syncs: view.syncs,
            last: view.last,
            last_at_ms: view
                .last_age_s
                .map(|age| read_ms.saturating_sub(i64::from(age).saturating_mul(1000))),
            ntp_last: view.ntp_last,
        }
    }
}

/// What `bound_ppb` adds to the error over `elapsed_ms`, rounded up as the
/// feeder rounds its own.
fn grown_ms(elapsed_ms: i64, bound_ppb: u32) -> u32 {
    let elapsed = u128::try_from(elapsed_ms).unwrap_or(0);
    let grown = (elapsed * u128::from(bound_ppb)).div_ceil(1_000_000_000);
    u32::try_from(grown).unwrap_or(u32::MAX)
}

/// Each portion's time: from the drive's start or the mark before, to its own
/// mark, since the feeder counts one portion per rotor mark. Empty when the
/// marks kept do not add up to the portions it counted.
fn portion_times(entries: &[TraceEntry], completed: u8) -> Vec<u32> {
    let mut from = None;
    let mut times = Vec::new();
    for entry in entries {
        match *entry {
            TraceEntry::Motor {
                at_ms,
                motor: TraceMotor::Drive0 | TraceMotor::Drive1,
            } if from.is_none() => from = Some(at_ms),
            TraceEntry::Mark {
                at_ms, high: true, ..
            } => {
                let Some(start) = from else { return Vec::new() };
                times.push(at_ms.saturating_sub(start));
                from = Some(at_ms);
            }
            _ => {}
        }
    }
    if times.len() == usize::from(completed) {
        times
    } else {
        Vec::new()
    }
}

pub(crate) const fn refusal_of(refusal: Refusal) -> FeederRefusal {
    match refusal {
        Refusal::Forbidden => FeederRefusal::Forbidden,
        Refusal::Stale { current } => FeederRefusal::Stale { current: current.0 },
        Refusal::Busy => FeederRefusal::Busy,
        Refusal::Unavailable => FeederRefusal::Unavailable,
        Refusal::Invalid => FeederRefusal::Invalid,
        Refusal::NotWritable => FeederRefusal::NotWritable,
        Refusal::Full => FeederRefusal::Full,
        Refusal::NotFound => FeederRefusal::NotFound,
        Refusal::Muted => FeederRefusal::Muted,
        Refusal::Time => FeederRefusal::Time,
        Refusal::Fault => FeederRefusal::Fault,
        Refusal::Update(reason) => FeederRefusal::Update { reason },
        Refusal::Unverified => FeederRefusal::Unverified,
        Refusal::Portions { asked } => FeederRefusal::Portions { asked },
        Refusal::NoClock => FeederRefusal::NoClock,
    }
}

fn call_of(view: CallView) -> CallInfo {
    CallInfo {
        writable: view.writable,
        clip: view.clip,
        ms: view.clip.ms(),
    }
}

fn client_of(client: Client) -> ClientView {
    ClientView {
        fingerprint: wellfed_session::hex::Lower(&client.key.0[..4]).to_string(),
        key: client.key.0.to_vec(),
        label: client.label,
        role: client.role,
        you: client.you,
    }
}

const fn calibration_of(view: CalibrationView) -> CalibrationInfo {
    CalibrationInfo {
        generation: view.generation.0,
        writable: view.writable,
        portion_mg: view.calibration.portion_mg,
        bowl: view.calibration.bowl,
        phase: view.phase,
    }
}

fn invite_of(invite: wellfed_model::Invite, identity: [u8; 32], role: ClientRole) -> InviteView {
    InviteView {
        text: Invite {
            feeder: identity,
            id: invite.id,
            secret: invite.secret.0,
        }
        .text(),
        seconds: invite.seconds,
        role,
    }
}

fn reply_of(answer: Answer) -> FeederReply {
    match answer {
        Answer::Done { generation } => FeederReply::Done {
            generation: generation.map(|generation| generation.0),
        },
        Answer::Settings(settings) => FeederReply::Settings {
            settings: settings_view(&settings),
        },
        Answer::Time(time) => FeederReply::Time {
            time: TimeRead::just_read(time).info(Instant::now()),
        },
        Answer::Plans(plans) => FeederReply::Plans {
            plans: plans_view(&plans),
        },
        Answer::Clients(clients) => FeederReply::Clients {
            clients: clients.into_iter().map(client_of).collect(),
        },
        Answer::Calibration(view) => FeederReply::Calibration {
            calibration: calibration_of(view),
        },
        Answer::Call(view) => FeederReply::Call {
            call: call_of(view),
        },
        Answer::Stats(stats) => FeederReply::Stats { stats },
        Answer::Update(view) => FeederReply::Update {
            update: UpdateInfo {
                phase: view.phase,
                bank: view.bank,
                min_version: view.min_version,
                version: view.version,
                image_len: view.image_len,
                written: view.written,
            },
        },
        Answer::Refused(refusal) => FeederReply::Refused {
            refusal: refusal_of(refusal),
        },
        Answer::Invite(_)
        | Answer::Status(_)
        | Answer::Feeding { .. }
        | Answer::FeedTrace(_)
        | Answer::Bank1(_) => FeederReply::Unexpected,
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn read_at(read_at: Timestamp, read_mono: Instant) -> TimeRead {
        TimeRead::new(
            TimeView {
                generation: Generation(1),
                writable: true,
                zone: None,
                ntp: NtpServer::Off,
                clock_ms: Some(1_000_000),
                error_ms: Some(50),
                offset_s: None,
                rate_ppb: 0,
                // 100 parts per million.
                rate_bound_ppb: 100_000,
                learned: true,
                syncs: 1,
                last: None,
                last_age_s: Some(600),
                ntp_last: None,
            },
            read_at,
            read_mono,
        )
    }

    fn at_ms(ms: i64) -> Timestamp {
        Timestamp::from_millisecond(ms).expect("a test's moment is in range")
    }

    #[test]
    fn the_set_time_stays_while_the_clock_and_its_error_run_on() {
        let read = Instant::now();
        let time = read_at(at_ms(2_000_000_000), read);

        let first = time.info(read);
        assert_eq!(first.clock_ms, Some(1_000_000));
        assert_eq!(first.error_ms, Some(50));
        assert_eq!(first.last_at_ms, Some(2_000_000_000 - 600_000));

        // The card built again eight minutes on, as the phone saw it creep.
        let later = time.info(read + Duration::from_secs(480));
        assert_eq!(later.last_at_ms, first.last_at_ms);
        assert_eq!(later.clock_ms, Some(1_000_000 + 480_000));
        assert_eq!(later.error_ms, Some(50 + 48));
    }

    /// A second of 16-bit mono WAV at 8 kHz, loud enough to keep.
    fn recording() -> Vec<u8> {
        let samples: Vec<u8> = (0..8_000i16)
            .flat_map(|n| (if n % 16 < 8 { 8_000i16 } else { -8_000 }).to_le_bytes())
            .collect();
        let mut file = b"RIFF\0\0\0\0WAVEfmt ".to_vec();
        file.extend(16u32.to_le_bytes());
        file.extend(1u16.to_le_bytes());
        file.extend(1u16.to_le_bytes());
        file.extend(8_000u32.to_le_bytes());
        file.extend(16_000u32.to_le_bytes());
        file.extend(2u16.to_le_bytes());
        file.extend(16u16.to_le_bytes());
        file.extend(b"data");
        file.extend(u32::try_from(samples.len()).unwrap().to_le_bytes());
        file.extend(samples);
        file
    }

    #[tokio::test]
    async fn an_upload_is_not_sent_only_when_none_of_it_went_out() {
        let wav = recording();

        // The session ended first: the Dart side may wait and send it.
        let (commands, receiver) = mpsc::unbounded_channel();
        drop(receiver);
        assert!(matches!(
            upload_call(&commands, &wav).await,
            FeederReply::NotSent
        ));

        // The start went out and was answered, and then the session ended.
        let (commands, mut receiver) = mpsc::unbounded_channel();
        let feeder = async move {
            let Some(Command::Ask { reply, .. }) = receiver.recv().await else {
                panic!("the upload's start comes first");
            };
            reply
                .send(Some(Answer::Done { generation: None }))
                .expect("the upload is waiting");
            // The next op, dropped unsent with the session.
            drop(receiver.recv().await);
        };
        let (reply, ()) = tokio::join!(upload_call(&commands, &wav), feeder);
        assert!(matches!(reply, FeederReply::Unanswered));
    }

    #[test]
    fn a_moment_before_the_read_passes_no_time() {
        let read = Instant::now() + Duration::from_secs(60);
        let time = read_at(at_ms(2_000_000_000), read);

        let info = time.info(read - Duration::from_secs(30));
        assert_eq!(info.clock_ms, Some(1_000_000));
        assert_eq!(info.error_ms, Some(50));
        assert_eq!(info.last_at_ms, Some(2_000_000_000 - 600_000));
    }

    #[test]
    fn the_growth_rounds_up_and_saturates() {
        assert_eq!(grown_ms(0, 100_000), 0);
        assert_eq!(grown_ms(1, 100_000), 1);
        assert_eq!(grown_ms(i64::MAX, u32::MAX), u32::MAX);
    }

    #[test]
    fn a_portion_runs_from_the_mark_before_and_marks_must_add_up() {
        let mark = |at_ms, high| TraceEntry::Mark {
            at_ms,
            edges: 0,
            high,
        };
        let entries = [
            TraceEntry::Motor {
                at_ms: 100,
                motor: TraceMotor::Drive0,
            },
            mark(9_100, true),
            mark(9_300, false),
            mark(18_200, true),
        ];

        assert_eq!(portion_times(&entries, 2), [9_000, 9_100]);
        assert!(portion_times(&entries, 3).is_empty());
    }
}
