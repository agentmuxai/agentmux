// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Window lifecycle reducer handlers. Extracted from reducer/mod.rs
//! in task #182 PR-C for navigability.
//!
//! Handles ReportWindowOpened, ReportWindowClosed, and the
//! ReportBackendWindowId{Registered,Unregistered} pair.

use agentmux_common::ipc::{Event, WindowKind};

use crate::reducer::Ctx;
use crate::state::{State, WindowMirror};

use agentmux_common::ipc::HwndDriftKind;

/// Phase B.5 (window_id_map step a) — record the host-reported
/// label → backend_window_id mapping. Idempotent on duplicate
/// label (overwrites with the new ID and emits a fresh event so
/// subscribers see the latest mapping).
pub(super) fn handle_report_backend_window_id_registered(
    state: &mut State,
    label: String,
    window_id: String,
) -> Vec<Event> {
    state
        .backend_window_ids
        .insert(label.clone(), window_id.clone());
    let v = state.bump_version();
    vec![Event::BackendWindowIdRegistered {
        label,
        window_id,
        version: v,
    }]
}

/// Phase B.5 (window_id_map step a) — drop the host-reported label
/// from the map. Strict pairing: emits `BackendWindowIdUnregistered`
/// only when the label was present (mirrors `WindowClosed` and
/// `PoolWindowRemoved` semantics — codex P2 PR #577 round-2).
pub(super) fn handle_report_backend_window_id_unregistered(
    state: &mut State,
    label: String,
) -> Vec<Event> {
    let removed = state.backend_window_ids.remove(&label);
    let Some(window_id) = removed else {
        return vec![];
    };
    let v = state.bump_version();
    vec![Event::BackendWindowIdUnregistered {
        label,
        window_id,
        version: v,
    }]
}

/// Phase B.4 — record a host-reported window opening in the launcher's
/// read-only mirror. Idempotent on duplicate opens (same label twice
/// in a row): the second insert overwrites with fresh metadata and
/// emits a fresh event. Subscribers must tolerate seeing the same
/// label twice; cleaner once B.5 makes the launcher authoritative.
///
/// Phase B.5: also assigns an authoritative instance number from
/// `state.instance_registry` and emits `WindowInstanceAssigned`.
/// "main" is pre-seeded with 1; other labels get the next value of
/// `next_instance_num`. Re-opens of an existing label preserve the
/// original number — instance numbers are stable per-label-per-run.
pub(super) fn handle_report_window_opened(
    state: &mut State,
    ctx: &Ctx,
    label: String,
    kind: WindowKind,
    parent_label: Option<String>,
) -> Vec<Event> {
    // PR #664 codex P1 round 2 — drain-on-WindowOpened RESTORED as
    // best-effort fallback. The explicit `ReportHwndOpened(Some(label))`
    // from `client.rs::on_after_created` remains the AUTHORITATIVE
    // link, and `apply_hwnd_opened` REPAIRS stale links it finds.
    // But that explicit dispatch is gated on `hwnd_val != 0` host-side;
    // if the HWND can't be resolved at on_after_created time from
    // either of the 2 sources (Views, host), the explicit dispatch
    // is skipped and the mirror would otherwise stay permanently
    // unlinked, breaking WRR drift detection AND orphan-destroy
    // reconciliation (no WindowClosed when OS destroys the HWND →
    // permanent ghost InstancePanel rows).
    //
    // (PR #664 round 4 dropped a 3rd fallback `find_own_top_level_window`
    // because it returns the FIRST visible window in the process —
    // some other window's HWND in a multi-window session — which
    // would corrupt other labels' mirrors via the `Repaired` arm.
    // See client.rs::on_after_created comment for details.)
    //
    // The drain provides a fallback link from `pending_hwnds`. If the
    // drain picks the WRONG HWND (the original burst-create race), the
    // subsequent `apply_hwnd_opened` call from on_after_created will
    // detect the mismatch and REPAIR — see the `Repaired` arm there.
    // Net: best-effort link via drain, authoritative repair via
    // explicit dispatch. The combination addresses both the
    // hwnd_val=0 case and the burst-create race.
    const PENDING_AGE_LIMIT_MS: u64 = 2_000;
    let drained_hwnd: Option<u64> = state
        .pending_hwnds
        .iter()
        .filter(|(_, p)| p.label_hint.is_none())
        .filter(|(_, p)| ctx.now_ms.saturating_sub(p.arrived_at_ms) <= PENDING_AGE_LIMIT_MS)
        .max_by_key(|(_, p)| p.arrived_at_ms)
        .map(|(hwnd, _)| *hwnd);
    if let Some(hwnd) = drained_hwnd {
        state.pending_hwnds.remove(&hwnd);
    }

    // Drift-storm fix (PR #708 round 3) — if this open is the back
    // half of a tear-off promote (host emit order is `pool_removed →
    // pool_promoted → window_opened`), `handle_report_pool_window_promoted`
    // recorded the label in `just_promoted_labels`. Initialize the new
    // mirror with `foregrounded_since_open: true` so the open-transient
    // corrective logic doesn't re-fire `HiddenSinceOpen` on the
    // subsequent HWND repositioning. See state.rs::just_promoted_labels.
    let was_just_promoted = state.just_promoted_labels.remove(&label);
    // Duplicate-open monotonicity (codex P2 PR #708 round 3):
    // `foregrounded_since_open` is monotonic (false → true, never
    // resets within a window's lifetime). On duplicate opens (the
    // existing handler overwrites the mirror wholesale), OR the prior
    // value in so the flag isn't reset.
    let prior_foregrounded = state
        .windows
        .get(&label)
        .map(|m| m.foregrounded_since_open)
        .unwrap_or(false);

    state.windows.insert(
        label.clone(),
        WindowMirror {
            label: label.clone(),
            kind,
            parent_label: parent_label.clone(),
            opened_at: ctx.now_rfc3339.clone(),
            // Best-effort drain above; authoritative explicit
            // ReportHwndOpened from on_after_created arrives a few
            // ms later via `apply_hwnd_opened` and REPAIRS any wrong
            // link the drain picked.
            hwnd: drained_hwnd,
            visible: false,
            iconic: false,
            last_rect: None,
            last_foreground_at_ms: None,
            foregrounded_since_open: was_just_promoted || prior_foregrounded,
            hidden_since_open_emitted: false,
        },
    );
    let mut out = Vec::with_capacity(2);
    let v = state.bump_version();
    out.push(Event::WindowOpened {
        label: label.clone(),
        kind,
        parent_label,
        version: v,
    });

    // Assign instance number if this label isn't already in the
    // registry. Re-opens of an existing label keep the original
    // number — matches host's `WindowInstanceRegistry` semantics
    // where a label is only registered once per session.
    let num = if let Some(existing) = state.instance_registry.get(&label).copied() {
        existing
    } else {
        let n = state.next_instance_num;
        state.instance_registry.insert(label.clone(), n);
        state.next_instance_num += 1;
        n
    };
    let v = state.bump_version();
    out.push(Event::WindowInstanceAssigned { label, num, version: v });
    out
}

/// Phase B.4 — drop a host-reported window from the mirror. Returns
/// `Event::WindowClosed` only when the label was actually in the
/// mirror; an unknown-label close is a silent no-op (codex P2 PR
/// #577 round-2). Without this gate, a `ReportWindowClosed` for a
/// label the launcher never saw (e.g. a pool window that was popped
/// from the queue but failed HWND validation in
/// `promote_pool_window` — the orphan window's eventual
/// `on_before_close` reaches us without a matching open) would
/// emit an unpaired `WindowClosed` broadcast and break subscribers
/// that assume open/close pairing.
///
/// Phase B.5 — also drops the label from `instance_registry` and
/// emits `WindowInstanceReleased` if a number was assigned.
/// `next_instance_num` is NOT decremented — instance numbers are
/// monotonic per-launcher-run.
pub(super) fn handle_report_window_closed(state: &mut State, label: String) -> Vec<Event> {
    let was_present = state.windows.remove(&label).is_some();
    // Drift-storm fix cleanup — drop any orphaned just-promoted entry.
    // Bounded leak protection for the (rare) case where promote was
    // emitted but the matching `ReportWindowOpened` never arrived
    // (host crash mid-tear-off, etc.).
    state.just_promoted_labels.remove(&label);
    if !was_present {
        // Silent: only emit when the close pairs with a known open.
        return vec![];
    }
    let mut out = Vec::with_capacity(4);
    let v = state.bump_version();
    out.push(Event::WindowClosed {
        label: label.clone(),
        version: v,
        // Clean close — host ran on_before_close before sending
        // ReportWindowClosed. F.6 saga is safe to trigger.
        crash_detected: false,
    });
    if let Some(num) = state.instance_registry.remove(&label) {
        let v = state.bump_version();
        out.push(Event::WindowInstanceReleased { label: label.clone(), num, version: v });
    }

    // Phase B.9.3 — OrphanInstance transition. The label we just
    // removed was the LAST user-visible window (state.windows is
    // now empty). If a Host is still registered as Running, its
    // own close path won't quit_message_loop because the warm
    // pool is keeping state.browsers non-empty. Emit drift +
    // saga-style HostShouldQuit so the host can reap pool and
    // quit cleanly. See B.9.3 in
    // docs/retro/next-steps-2026-04-29.md.
    if state.windows.is_empty() && super::connection::host_is_running(state) {
        let v_drift = state.bump_version();
        out.push(Event::HwndDriftDetected {
            kind: HwndDriftKind::OrphanInstance,
            label: Some(label),
            hwnd: None,
            detail: "Last user-visible window closed; host still alive (likely holding warm pool)"
                .to_string(),
            severity: agentmux_common::ipc::Severity::Warn,
            version: v_drift,
        });
        let v_quit = state.bump_version();
        out.push(Event::HostShouldQuit { version: v_quit });
    }
    out
}
