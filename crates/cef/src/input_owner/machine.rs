// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! One window's input owner: whether the keyboard goes to the app's own content
//! or to one browser pane's page. Pure, like the host reducer: no Win32, CEF,
//! locks, clock or logging. The platform adapter feeds it events, with the time,
//! and runs the effects it returns on the UI thread.
//! docs/specs/SPEC_BROWSER_PANE_INPUT_OWNER_2026_10_15.md §3.

use std::collections::BTreeMap;
use std::fmt;

/// A browser pane, by its block id.
pub type PaneId = String;

/// Who should, or does, get a window's keyboard.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Owner {
    App,
    Pane(PaneId),
}

/// Where the platform reports keyboard focus: in this window's app content, in
/// one of its panes, or anywhere else (another window or application).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Surface {
    App,
    Pane(PaneId),
    Elsewhere,
}

/// A pane as the machine needs to know it. Only a live pane can take focus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaneLife {
    Live,
    /// Laid out at 0×0: its tab or pane tab isn't the visible one.
    Collapsed,
    Closing,
}

/// What the machine knows about where the keyboard is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Have {
    /// Nothing reported yet.
    Unknown,
    /// Last seen here.
    Owned(Surface),
    /// Asked for `target`, not yet seen there; `attempt` counts the asks.
    Pending { target: Owner, attempt: u8 },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// The user's selection, from the frontend.
    Select(Owner),
    /// A click in the pane's page.
    PanePointerDown(PaneId),
    /// A click in the app's own content (where the platform reports it: macOS,
    /// Phase 4; Windows doesn't see it, the frontend's selection stands in).
    #[cfg_attr(target_os = "windows", allow(dead_code))]
    AppPointerDown,
    /// The platform reported focus arriving at a surface of this window.
    FocusGained(Surface),
    Activated,
    Deactivated,
    WindowShown,
    PaneLive(PaneId),
    PaneShown(PaneId),
    PaneCollapsed(PaneId),
    PaneClosing(PaneId),
    PaneGone(PaneId),
    /// The posted check (`Effect::Verify`): where the platform reports focus now.
    /// `seq` is the check's own; only the latest one asked for counts.
    Verify { seq: u64, surface: Surface },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Give the keyboard to the app's own content.
    FocusApp,
    /// Give the keyboard to this pane's page.
    FocusPane(PaneId),
    /// Put every live, shown pane above the app's own child windows, so a visible
    /// pane takes its wheel and clicks.
    RaisePanes,
    /// Check where focus is after this long, and send `Event::Verify` with this
    /// `seq`. A later check makes this one stale.
    Verify { seq: u64, after_ms: u64 },
    /// Something the machine couldn't make true, for the log.
    Warn(&'static str),
}

/// How long after asking for focus the machine checks it arrived.
pub const VERIFY_AFTER_MS: u64 = 50;
/// The fewest milliseconds between two corrections of a page's own `focus()`, per
/// window, so a page that refocuses itself in a loop can't storm the UI thread.
pub const RESTORE_MIN_INTERVAL_MS: u64 = 100;
/// Asks for one owner before the machine accepts where focus is and warns.
pub const MAX_ATTEMPTS: u8 = 3;
/// How long a selection that can't be applied yet stays applicable. The same as
/// macOS's parked pane-key request (`PENDING_PANE_KEY_TTL`, #4958): a request
/// applied later than this took the keyboard from whatever the user moved on to.
pub const PARKED_WANT_TTL_MS: u64 = 1500;

/// One top-level window's input state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowInput {
    /// The window is the foreground window. Focus effects activate a window, so
    /// an inactive one gets none.
    pub active: bool,
    /// The user's selection, and when it was made.
    pub want: Owner,
    pub want_at_ms: u64,
    /// The selection couldn't be applied when it was made (its pane wasn't live
    /// yet) and may still be, by the pane going live within `PARKED_WANT_TTL_MS`.
    /// Hiding or closing the pane releases it.
    pub parked: bool,
    pub have: Have,
    pub panes: BTreeMap<PaneId, PaneLife>,
    /// When a page's own focus was last corrected, and whether a correction was
    /// put off by the rate limit and is still owed.
    pub last_restore_ms: Option<u64>,
    pub restore_due: bool,
    /// The latest check asked for. A check that comes back with an older `seq`
    /// belongs to an earlier request and is ignored.
    pub verify_seq: u64,
}

impl Default for WindowInput {
    fn default() -> Self {
        Self {
            active: false,
            want: Owner::App,
            want_at_ms: 0,
            parked: false,
            have: Have::Unknown,
            panes: BTreeMap::new(),
            last_restore_ms: None,
            restore_due: false,
            verify_seq: 0,
        }
    }
}

impl Owner {
    pub fn surface(&self) -> Surface {
        match self {
            Owner::App => Surface::App,
            Owner::Pane(b) => Surface::Pane(b.clone()),
        }
    }
}

fn focus_effect(target: &Owner) -> Effect {
    match target {
        Owner::App => Effect::FocusApp,
        Owner::Pane(b) => Effect::FocusPane(b.clone()),
    }
}

impl WindowInput {
    /// `owner` if it can take focus now, else the app.
    pub fn usable(&self, owner: &Owner) -> Owner {
        match owner {
            Owner::Pane(b) if self.panes.get(b) == Some(&PaneLife::Live) => owner.clone(),
            _ => Owner::App,
        }
    }

    /// Whether the keyboard is with `b`, or has been asked for it.
    fn involves(&self, b: &str) -> bool {
        match &self.have {
            Have::Owned(Surface::Pane(p)) => p == b,
            Have::Pending { target: Owner::Pane(p), .. } => p == b,
            _ => false,
        }
    }

    /// Ask for a check after `after_ms`; any earlier check still out becomes stale.
    fn schedule_verify(&mut self, after_ms: u64) -> Effect {
        self.verify_seq += 1;
        Effect::Verify { seq: self.verify_seq, after_ms }
    }

    fn request(&mut self, target: Owner, attempt: u8) -> Vec<Effect> {
        let effect = focus_effect(&target);
        self.have = Have::Pending { target, attempt };
        vec![effect, self.schedule_verify(VERIFY_AFTER_MS)]
    }

    /// How long until a page's own focus may be corrected again; 0 if now.
    fn restore_wait(&self, now_ms: u64) -> u64 {
        match self.last_restore_ms {
            Some(last) => RESTORE_MIN_INTERVAL_MS.saturating_sub(now_ms.saturating_sub(last)),
            None => 0,
        }
    }

    fn choose(&mut self, owner: Owner, now_ms: u64) {
        self.want = owner;
        self.want_at_ms = now_ms;
        self.parked = false;
    }

    /// Apply the user's selection: ask for it unless it is already there or asked for.
    fn apply_selection(&mut self) -> Vec<Effect> {
        if !self.active {
            return vec![];
        }
        let target = self.usable(&self.want.clone());
        // A pane that can't take focus yet: the app has it meanwhile, and the
        // selection waits for the pane to go live.
        self.parked = target != self.want;
        match &self.have {
            Have::Owned(s) if *s == target.surface() => {
                let check = self.schedule_verify(0);
                vec![check]
            }
            Have::Pending { target: t, .. } if *t == target => vec![],
            _ => self.request(target, 0),
        }
    }

    /// Correct a page that took the keyboard on its own, at most once per
    /// `RESTORE_MIN_INTERVAL_MS`; a correction inside that time is put off, not dropped.
    fn restore(&mut self, observed: Surface, now_ms: u64) -> Vec<Effect> {
        let wait = self.restore_wait(now_ms);
        if wait > 0 {
            self.have = Have::Owned(observed);
            self.restore_due = true;
            return vec![self.schedule_verify(wait)];
        }
        self.last_restore_ms = Some(now_ms);
        self.restore_due = false;
        let target = self.usable(&self.want.clone());
        self.request(target, 0)
    }

    /// The pane `b` stopped being able to take focus: if it had the keyboard, the
    /// app takes it. An inactive window can't be given focus; activation decides.
    /// A selection of `b` waiting to be applied is released: showing `b` again
    /// doesn't take the keyboard without a fresh selection.
    fn pane_unusable(&mut self, b: &str) -> Vec<Effect> {
        if self.want == Owner::Pane(b.to_string()) {
            self.parked = false;
        }
        if !self.involves(b) {
            return vec![];
        }
        if self.active {
            self.request(Owner::App, 0)
        } else {
            self.have = Have::Unknown;
            vec![]
        }
    }

    /// A pane became live or shown: apply a recent selection of it that was
    /// waiting for it.
    fn pane_usable(&mut self, b: &str, now_ms: u64) -> Vec<Effect> {
        let wanted = self.want == Owner::Pane(b.to_string());
        let recent = now_ms.saturating_sub(self.want_at_ms) <= PARKED_WANT_TTL_MS;
        if wanted && self.parked && recent && self.active && !self.involves(b) {
            self.parked = false;
            self.request(Owner::Pane(b.to_string()), 0)
        } else {
            vec![]
        }
    }

    pub fn step(&mut self, event: Event, now_ms: u64) -> Vec<Effect> {
        // A selection that waited too long is dropped: the app has had the
        // keyboard meanwhile, and nothing later (a pane going live, the window
        // being activated) may apply it.
        if self.parked && now_ms.saturating_sub(self.want_at_ms) > PARKED_WANT_TTL_MS {
            self.parked = false;
            self.want = Owner::App;
        }
        match event {
            Event::Select(owner) => {
                self.choose(owner, now_ms);
                self.apply_selection()
            }
            Event::PanePointerDown(b) => {
                self.choose(Owner::Pane(b), now_ms);
                self.apply_selection()
            }
            Event::AppPointerDown => {
                self.choose(Owner::App, now_ms);
                self.apply_selection()
            }
            Event::FocusGained(s) => self.focus_gained(s, now_ms),
            Event::Activated => {
                self.active = true;
                let target = self.usable(&self.want.clone());
                let mut effects = vec![Effect::RaisePanes];
                effects.extend(self.request(target, 0));
                effects
            }
            Event::Deactivated => {
                self.active = false;
                vec![]
            }
            Event::WindowShown => vec![Effect::RaisePanes],
            Event::PaneLive(b) | Event::PaneShown(b) => {
                self.panes.insert(b.clone(), PaneLife::Live);
                self.pane_usable(&b, now_ms)
            }
            Event::PaneCollapsed(b) => {
                self.panes.insert(b.clone(), PaneLife::Collapsed);
                self.pane_unusable(&b)
            }
            Event::PaneClosing(b) => {
                self.panes.insert(b.clone(), PaneLife::Closing);
                self.pane_closed(&b, now_ms)
            }
            Event::PaneGone(b) => {
                let effects = self.pane_closed(&b, now_ms);
                self.panes.remove(&b);
                effects
            }
            Event::Verify { seq, surface } => {
                if seq != self.verify_seq {
                    // A check for an earlier request: a later one is on its way.
                    return vec![];
                }
                self.verify(surface, now_ms)
            }
        }
    }

    fn pane_closed(&mut self, b: &str, now_ms: u64) -> Vec<Effect> {
        if self.want == Owner::Pane(b.to_string()) {
            self.choose(Owner::App, now_ms);
        }
        self.pane_unusable(b)
    }

    fn focus_gained(&mut self, s: Surface, now_ms: u64) -> Vec<Effect> {
        if !self.active || s == Surface::Elsewhere {
            self.have = Have::Owned(s);
            return vec![];
        }
        match &self.have {
            Have::Pending { target, .. } if target.surface() == s => {
                self.have = Have::Owned(s);
                return vec![];
            }
            Have::Owned(current) if *current == s => return vec![],
            _ => {}
        }
        match &s {
            // A drift toward the app is never reversed (the user may have just
            // clicked it, ahead of the frontend's selection).
            Surface::App => {
                self.have = Have::Owned(s);
                vec![]
            }
            // The selected pane taking it, however it happened, is what the user wants.
            Surface::Pane(b) if self.usable(&self.want) == Owner::Pane(b.clone()) => {
                self.have = Have::Owned(s);
                vec![]
            }
            // A page took the keyboard on its own.
            Surface::Pane(_) => self.restore(s, now_ms),
            Surface::Elsewhere => unreachable!("handled above"),
        }
    }

    fn verify(&mut self, s: Surface, now_ms: u64) -> Vec<Effect> {
        if !self.active {
            self.have = Have::Owned(s);
            self.restore_due = false;
            return vec![];
        }
        if self.restore_due {
            let target = self.usable(&self.want.clone());
            if s == target.surface() {
                self.restore_due = false;
                self.have = Have::Owned(s);
                return vec![];
            }
            // The owed correction keeps to the same rate as any other.
            let wait = self.restore_wait(now_ms);
            if wait > 0 {
                return vec![self.schedule_verify(wait)];
            }
            self.restore_due = false;
            self.last_restore_ms = Some(now_ms);
            return self.request(target, 0);
        }
        match self.have.clone() {
            Have::Pending { target, attempt } => {
                if s == target.surface() {
                    self.have = Have::Owned(s);
                    return vec![];
                }
                // The target may have stopped being usable meanwhile.
                let usable = self.usable(&target);
                if usable != target {
                    return self.request(usable, 0);
                }
                if attempt + 1 < MAX_ATTEMPTS {
                    self.request(target, attempt + 1)
                } else {
                    self.have = Have::Owned(s);
                    vec![Effect::Warn("focus did not reach the requested owner")]
                }
            }
            Have::Owned(current) => {
                if current != s {
                    // Recorded, not reversed. Focus that moved to the app without
                    // the machine asking is the user's choice.
                    if s == Surface::App {
                        self.choose(Owner::App, now_ms);
                    }
                    self.have = Have::Owned(s);
                }
                vec![]
            }
            Have::Unknown => {
                self.have = Have::Owned(s);
                vec![]
            }
        }
    }
}

impl fmt::Display for Owner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Owner::App => write!(f, "app"),
            Owner::Pane(b) => write!(f, "pane:{}", short(b)),
        }
    }
}

impl fmt::Display for Surface {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Surface::App => write!(f, "app"),
            Surface::Pane(b) => write!(f, "pane:{}", short(b)),
            Surface::Elsewhere => write!(f, "elsewhere"),
        }
    }
}

impl fmt::Display for Have {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Have::Unknown => write!(f, "unknown"),
            Have::Owned(s) => write!(f, "{s}"),
            Have::Pending { target, attempt } => write!(f, "pending({target}#{attempt})"),
        }
    }
}

impl fmt::Display for Event {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Event::Select(o) => write!(f, "select({o})"),
            Event::PanePointerDown(b) => write!(f, "pane-pointer-down(pane:{})", short(b)),
            Event::AppPointerDown => write!(f, "app-pointer-down"),
            Event::FocusGained(s) => write!(f, "focus-gained({s})"),
            Event::Activated => write!(f, "activated"),
            Event::Deactivated => write!(f, "deactivated"),
            Event::WindowShown => write!(f, "window-shown"),
            Event::PaneLive(b) => write!(f, "pane-live(pane:{})", short(b)),
            Event::PaneShown(b) => write!(f, "pane-shown(pane:{})", short(b)),
            Event::PaneCollapsed(b) => write!(f, "pane-collapsed(pane:{})", short(b)),
            Event::PaneClosing(b) => write!(f, "pane-closing(pane:{})", short(b)),
            Event::PaneGone(b) => write!(f, "pane-gone(pane:{})", short(b)),
            Event::Verify { seq, surface } => write!(f, "verify#{seq}({surface})"),
        }
    }
}

impl fmt::Display for Effect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Effect::FocusApp => write!(f, "focus-app"),
            Effect::FocusPane(b) => write!(f, "focus-pane:{}", short(b)),
            Effect::RaisePanes => write!(f, "raise-panes"),
            Effect::Verify { seq, after_ms } => write!(f, "verify#{seq}+{after_ms}ms"),
            Effect::Warn(reason) => write!(f, "warn({reason})"),
        }
    }
}

/// A block id as the logs show it: its first 8 characters.
fn short(id: &str) -> &str {
    id.get(..8).unwrap_or(id)
}

impl WindowInput {
    /// One line for the `[input]` log: `active want=… have=…`.
    pub fn describe(&self) -> String {
        format!(
            "{} want={}{} have={}{}",
            if self.active { "active" } else { "inactive" },
            self.want,
            if self.parked { "(parked)" } else { "" },
            self.have,
            if self.restore_due { " restore-due" } else { "" },
        )
    }
}

#[cfg(test)]
#[path = "machine_tests.rs"]
mod tests;
