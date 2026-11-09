//! What the pass measures and what the `Stats` op reads back. The
//! statistics ([`Keeper`]) take the link, the food beam and the battery's
//! history each pass; the live samples go to subscribers (power each time
//! the supply is sampled; Wi-Fi signal, the heap, the loop's pass times,
//! the gateway's round trip and the access point every 30 s); and the
//! heavy `Stats` sections are built here, out of line.

use wellfed_proto::api::{
    self as proto, Battery, Metric, Power, Sample, Stats, Supply, TASKS_MAX, TaskStack as WireTask,
};

use super::super::Feeds;
use super::super::platform::{Platform, TaskCount, TaskStack};
use super::{DeviceView, Serve};
use crate::stats::Keeper;

/// Wi-Fi signal and, for subscribers with the statistics, the heap, the
/// loop, the gateway and the access point.
const RSSI_MS: u32 = 30_000;

impl<S: Copy> Serve<S> {
    /// What the statistics take from this pass: the link, the food beam
    /// and the battery's history.
    pub(super) fn keep_stats<P: Platform<Socket = S>, F: Feeds>(
        &mut self,
        platform: &mut P,
        feeds: &F,
        now_ms: u32,
    ) {
        let stats = &mut self.stats;
        stats.link(platform.link());
        if let Some(present) = feeds.food() {
            stats.food(present);
        }
        stats.tick(now_ms);
    }

    pub(super) fn samples<P: Platform<Socket = S>>(
        &mut self,
        platform: &mut P,
        now_ms: u32,
        supply: Supply,
    ) {
        if wellfed_core::deadline_passed(now_ms, self.next_power) {
            self.next_power = now_ms.wrapping_add(P::POWER_SAMPLE_MS);
            if let Some(power) = platform.power(now_ms) {
                // The history keeps only readings a pack can give.
                let battery_mv = match supply.battery {
                    Battery::Reading { mv, .. } => Some(u32::from(mv)),
                    Battery::Absent
                    | Battery::Implausible { .. }
                    | Battery::Unread
                    | Battery::Stale { .. } => None,
                };
                self.stats.power(
                    now_ms,
                    Power {
                        battery_mv,
                        ..power
                    },
                );
                self.server.sample(Sample {
                    at_ms: now_ms,
                    metric: Metric::Power(super::super::supply::power(power, supply.battery)),
                });
            }
            self.server.sample(Sample {
                at_ms: now_ms,
                metric: Metric::Supply(supply),
            });
        }
        if wellfed_core::deadline_passed(now_ms, self.next_rssi) {
            self.next_rssi = now_ms.wrapping_add(RSSI_MS);
            // The writer saves the totals far less often than this.
            platform.hand_counts(self.stats.counts(now_ms));
            self.server.sample(Sample {
                at_ms: now_ms,
                metric: Metric::Rssi(platform.link().rssi),
            });
            // Taken every window whether or not anyone subscribes, so each
            // covers the same 30 s.
            let (free, min) = platform.heap();
            for metric in [
                Metric::Heap { free, min },
                self.stats.loop_metric(),
                self.stats.gateway_metric(),
                self.stats.wifi_metric(),
            ] {
                self.server.sample(Sample {
                    at_ms: now_ms,
                    metric,
                });
            }
        }
    }
}

/// How long a walk of the task stacks answers memory requests: the walk
/// pauses the scheduler, so a client asking again and again repeats none.
const TASKS_MS: u32 = 5_000;

/// The task stacks as last walked.
pub struct TaskCache {
    stacks: [TaskStack; TASKS_MAX],
    count: TaskCount,
    at: Option<u32>,
}

impl TaskCache {
    pub(super) const fn new() -> Self {
        Self {
            stacks: [TaskStack::EMPTY; TASKS_MAX],
            count: TaskCount {
                listed: 0,
                total: 0,
            },
            at: None,
        }
    }

    /// The stacks, walked again once the last walk is [`TASKS_MS`] old.
    fn read<P: Platform>(&mut self, platform: &mut P, now_ms: u32) -> (&[TaskStack], u32) {
        if self.at.is_none_or(|at| now_ms.wrapping_sub(at) >= TASKS_MS) {
            self.count = platform.task_stacks(&mut self.stacks);
            self.at = Some(now_ms);
        }
        let listed = self.count.listed.min(TASKS_MAX);
        (&self.stacks[..listed], self.count.total)
    }
}

impl<P: Platform, F: Feeds> DeviceView<'_, P, F> {
    // The heavy `Stats` sections, each out of line so the totals it copies and
    // the section it builds stay out of `Device::stats`'s frame, which every
    // other section pays for too.

    #[inline(never)]
    pub(super) fn counters_section(&mut self, now_ms: u32) -> Stats<'_> {
        let stored = self.platform.totals();
        Stats::Counters(self.stats.counters(now_ms, stored.as_ref()))
    }

    #[inline(never)]
    pub(super) fn memory_section(&mut self, now_ms: u32) -> Stats<'_> {
        let (heap_free, heap_min) = self.platform.heap();
        let (stacks, tasks_total) = self.tasks.read(&mut *self.platform, now_ms);
        let mut tasks = [None; TASKS_MAX];
        for (out, task) in tasks.iter_mut().zip(stacks) {
            *out = Some(WireTask {
                name: task.name(),
                free_min: task.free_min,
            });
        }
        Stats::Memory(proto::Memory {
            heap_free,
            heap_min,
            tasks,
            tasks_total,
        })
    }

    #[inline(never)]
    pub(super) fn flash_section(&mut self) -> Stats<'_> {
        let fill = self.platform.events_fill();
        let stored = self.platform.totals();
        Stats::Flash(Keeper::flash(
            self.platform.erases(),
            stored.as_ref(),
            fill.kept,
            fill.capacity,
        ))
    }
}
