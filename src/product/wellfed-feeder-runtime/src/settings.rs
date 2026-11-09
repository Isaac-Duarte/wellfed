//! The settings between the loop and the writer: the device's and the
//! feeder's own in one mailbox ([`SETTINGS`]), which the device API's
//! `settings` requests queue changes into
//! (`wellfed_device_runtime::asked::settings_request`) and the writer
//! publishes both records' view through. The device's record is in storage
//! sectors 2 and 3, and the feeder's ([`FEEDER_SETTINGS_BASE`]) in 48 and
//! 49. Nothing here writes flash.

use wellfed_device_api::mailbox::FeederSettings;
use wellfed_hal::settings::Mailbox;

/// The feeder's settings record's slots within the storage region.
pub const FEEDER_SETTINGS_BASE: u32 = wellfed_hal::board::storage::FEEDER_SETTINGS.first;

/// The firmware's settings mailbox.
pub static SETTINGS: Mailbox<FeederSettings> = Mailbox::new();
