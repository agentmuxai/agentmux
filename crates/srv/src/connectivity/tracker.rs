// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The reachability state machine and the probe schedule, with no I/O and no
//! clock of its own: every call takes `now_ms`, so tests drive time.

use std::collections::{BTreeMap, VecDeque};

use super::{ConnectivityStatus, Outcome, Reachability, Service, ServiceStatus};

/// Consecutive transport failures, with no answer between, that make a
/// service `Offline`.
pub(super) const OFFLINE_AFTER: u32 = 3;
/// How far back failures count toward keeping a service `Degraded`.
pub(super) const DEGRADED_WINDOW_MS: i64 = 60_000;
/// Failures within [`DEGRADED_WINDOW_MS`] that keep a service `Degraded`
/// after an answer; one stray reset is not worth a warning.
pub(super) const DEGRADED_MIN_FAILURES: usize = 2;
/// The first probe's delay after a service stops being `Online`, doubled
/// after each probe that still finds it down, up to [`PROBE_MAX_MS`].
pub(super) const PROBE_FIRST_MS: i64 = 5_000;
pub(super) const PROBE_MAX_MS: i64 = 60_000;

/// A probe that is scheduled or running, while its service isn't `Online`.
#[derive(Debug, Clone, PartialEq)]
struct ProbeSchedule {
    due_ms: i64,
    interval_ms: i64,
    /// The running probe's ticket; only its result is applied.
    in_flight: Option<u64>,
}

#[derive(Debug, Clone)]
struct Entry {
    state: Reachability,
    since_ms: i64,
    consecutive_failures: u32,
    /// When the latest transport failures happened, within
    /// [`DEGRADED_WINDOW_MS`]; no more than the count that matters.
    recent_failures: VecDeque<i64>,
    last_error: Option<String>,
    probe: Option<ProbeSchedule>,
}

impl Entry {
    fn new(now_ms: i64) -> Self {
        Self {
            state: Reachability::Online,
            since_ms: now_ms,
            consecutive_failures: 0,
            recent_failures: VecDeque::new(),
            last_error: None,
            probe: None,
        }
    }

    /// A service not checked yet, its first probe due now.
    fn unchecked(now_ms: i64) -> Self {
        let probe = Some(ProbeSchedule { due_ms: now_ms, interval_ms: PROBE_FIRST_MS, in_flight: None });
        Self { state: Reachability::Unknown, probe, ..Self::new(now_ms) }
    }

    fn forget_old_failures(&mut self, now_ms: i64) {
        while self.recent_failures.front().is_some_and(|t| now_ms - t > DEGRADED_WINDOW_MS) {
            self.recent_failures.pop_front();
        }
    }

    /// Applies one outcome; `probed` says it came from a probe, which runs
    /// only while the service is believed down, so its failure confirms it.
    fn apply(&mut self, outcome: Outcome, probed: bool, now_ms: i64) -> Reachability {
        self.forget_old_failures(now_ms);
        match outcome {
            Outcome::Answered => {
                self.consecutive_failures = 0;
                let mixed = self.state == Reachability::Degraded && self.recent_failures.len() >= DEGRADED_MIN_FAILURES;
                if mixed {
                    Reachability::Degraded
                } else {
                    Reachability::Online
                }
            }
            Outcome::Unreachable(error) => {
                self.consecutive_failures = self.consecutive_failures.saturating_add(1);
                self.recent_failures.push_back(now_ms);
                if self.recent_failures.len() > DEGRADED_MIN_FAILURES {
                    self.recent_failures.pop_front();
                }
                self.last_error = Some(error);
                if probed || self.consecutive_failures >= OFFLINE_AFTER || self.state == Reachability::Offline {
                    Reachability::Offline
                } else {
                    Reachability::Degraded
                }
            }
        }
    }

    /// Moves to `next`; returns whether the state changed. Leaving `Online`
    /// schedules the first probe, reaching it drops the schedule.
    fn enter(&mut self, next: Reachability, now_ms: i64) -> bool {
        if next == self.state {
            return false;
        }
        if next == Reachability::Online {
            self.probe = None;
            self.last_error = None;
        } else if self.state == Reachability::Online {
            self.probe = Some(ProbeSchedule { due_ms: now_ms + PROBE_FIRST_MS, interval_ms: PROBE_FIRST_MS, in_flight: None });
        }
        self.state = next;
        self.since_ms = now_ms;
        true
    }
}

/// Every service's reachability, and the internet's, derived from them.
#[derive(Debug)]
pub(super) struct Tracker {
    services: BTreeMap<Service, Entry>,
    internet: (Reachability, i64),
    revision: u64,
    next_ticket: u64,
}

impl Tracker {
    /// `first_revision` starts above any earlier srv's, so a window that
    /// outlives an srv restart still takes the new one's status as newer.
    pub(super) fn new(first_revision: u64, now_ms: i64) -> Self {
        Self { services: BTreeMap::new(), internet: (Reachability::Online, now_ms), revision: first_revision, next_ticket: 0 }
    }

    /// Lists `service` as not checked yet, with a probe due now, unless it
    /// has already reported. Returns the new status when it was added.
    pub(super) fn seed(&mut self, service: Service, now_ms: i64) -> Option<ConnectivityStatus> {
        if self.services.contains_key(&service) {
            return None;
        }
        self.services.insert(service, Entry::unchecked(now_ms));
        self.changed(true, now_ms)
    }

    /// An outcome from a request a subsystem made. Returns the new status
    /// when a state changed; a new error text alone waits for the next one.
    pub(super) fn report(&mut self, service: Service, outcome: Outcome, now_ms: i64) -> Option<ConnectivityStatus> {
        let entry = self.services.entry(service).or_insert_with(|| Entry::new(now_ms));
        let next = entry.apply(outcome, false, now_ms);
        let changed = entry.enter(next, now_ms);
        self.changed(changed, now_ms)
    }

    /// The probes due at `now_ms`, each with the ticket its result must
    /// carry; they count as running until [`Tracker::probe_finished`].
    pub(super) fn take_due_probes(&mut self, now_ms: i64) -> Vec<(Service, u64)> {
        let mut due = Vec::new();
        for (service, entry) in &mut self.services {
            if let Some(probe) = entry.probe.as_mut().filter(|p| p.in_flight.is_none() && p.due_ms <= now_ms) {
                self.next_ticket += 1;
                probe.in_flight = Some(self.next_ticket);
                due.push((service.clone(), self.next_ticket));
            }
        }
        due
    }

    /// When the next probe that isn't running is due, if any is scheduled.
    pub(super) fn next_probe_due(&self) -> Option<i64> {
        self.services
            .values()
            .filter_map(|e| e.probe.as_ref().filter(|p| p.in_flight.is_none()).map(|p| p.due_ms))
            .min()
    }

    /// A probe's result. Ignored unless it is the probe still expected: the
    /// service may have come back, or gone down again, while it ran.
    pub(super) fn probe_finished(&mut self, service: &Service, ticket: u64, outcome: Outcome, now_ms: i64) -> Option<ConnectivityStatus> {
        let entry = self.services.get_mut(service)?;
        if entry.probe.as_ref().and_then(|p| p.in_flight) != Some(ticket) {
            return None;
        }
        let next = entry.apply(outcome, true, now_ms);
        let changed = entry.enter(next, now_ms);
        if let Some(probe) = entry.probe.as_mut() {
            probe.in_flight = None;
            probe.interval_ms = (probe.interval_ms * 2).min(PROBE_MAX_MS);
            probe.due_ms = now_ms + probe.interval_ms;
        }
        self.changed(changed, now_ms)
    }

    /// The internet is `Offline` when two or more services are and none
    /// answers: one unreachable host is that host's problem (a block, a
    /// captive portal), not the connection's. One not checked yet says
    /// nothing either way.
    fn derive_internet(&self) -> Reachability {
        let offline = self.services.values().filter(|e| e.state == Reachability::Offline).count();
        let reachable = self.services.values().any(|e| matches!(e.state, Reachability::Online | Reachability::Degraded));
        if offline >= 2 && !reachable {
            Reachability::Offline
        } else {
            Reachability::Online
        }
    }

    fn changed(&mut self, service_changed: bool, now_ms: i64) -> Option<ConnectivityStatus> {
        let internet = self.derive_internet();
        let internet_changed = internet != self.internet.0;
        if internet_changed {
            self.internet = (internet, now_ms);
        }
        if !service_changed && !internet_changed {
            return None;
        }
        self.revision += 1;
        Some(self.status())
    }

    pub(super) fn status(&self) -> ConnectivityStatus {
        let mut services = vec![ServiceStatus {
            service: Service::INTERNET_ID.to_string(),
            label: Service::INTERNET_LABEL.to_string(),
            state: self.internet.0,
            since_ms: self.internet.1,
            last_error: None,
        }];
        services.extend(self.services.iter().map(|(service, e)| ServiceStatus {
            service: service.id(),
            label: service.label(),
            state: e.state,
            since_ms: e.since_ms,
            last_error: e.last_error.clone(),
        }));
        ConnectivityStatus { revision: self.revision, services }
    }
}

#[cfg(test)]
#[path = "tracker_tests.rs"]
mod tests;
