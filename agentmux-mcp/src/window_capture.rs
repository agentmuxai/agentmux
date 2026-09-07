// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Window capture and discovery, plus the audit log both write.
//!
//! Split out of `main.rs` (audit step 17,
//! `docs/reports/REPORT_DRY_AND_MODULARITY_AUDIT_2026_09_06.md`) — it was the
//! largest single cluster in that file and is one concern: find AgentMux
//! windows, decide what this agent is allowed to capture (`CaptureTier`),
//! capture it, and record what was captured.
//!
//! Bodies are unchanged from `main.rs`; the only edits were `pub(crate)` on
//! the items `call_tool` and the tests name. `main.rs` glob-imports this
//! module, so those call sites — and `mod tests`'s `use super::*` — resolve
//! exactly as they did before.

use std::time::Duration;

use anyhow::Result;
use serde_json::{json, Value};

use crate::agent_slug;

pub(crate) fn capture_window_dir() -> std::path::PathBuf {
    let base = std::env::var("AGENTMUX_DATA_HOME")
        .ok()
        .filter(|d| !d.is_empty())
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            dirs::home_dir()
                .unwrap_or_else(|| std::path::PathBuf::from("/"))
                .join(".agentmux")
        });
    base.join("tmp/capture-window")
}

pub(crate) const CAPTURE_RETENTION: std::time::Duration = std::time::Duration::from_secs(60 * 60);

/// Same bug class, same fix, as `agentmux-srv`'s `prune_old_screenshots`
/// (`ui_handlers.rs`, reagent P2, PR #2662) — reapplied here per reagent's
/// review of this tool's own PR (#2709 round 1), which found it wasn't
/// reused. Best-effort, on the write path, PNG-only.
pub(crate) fn prune_old_captures(dir: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let now = std::time::SystemTime::now();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("png") {
            continue;
        }
        let Ok(metadata) = entry.metadata() else { continue };
        let Ok(modified) = metadata.modified() else { continue };
        let Ok(age) = now.duration_since(modified) else { continue };
        if age > CAPTURE_RETENTION {
            let _ = std::fs::remove_file(&path);
        }
    }
}

/// Max ancestor hops to walk from this MCP process before giving up —
/// bounds the fallback search (see `own_instance_pids`'s doc comment for why
/// this is a fallback, not the primary signal), while never walking so far
/// up the tree (e.g. to `explorer.exe`) that "same instance" degrades into
/// "everything on the desktop."
pub(crate) const OWN_INSTANCE_ANCESTOR_HOPS: usize = 8;

/// PIDs `CaptureWindow` must treat as "this agent's own AgentMux instance"
/// and always exclude — reagent P0 (PR #2709 round 3).
///
/// **Primary signal: `AGENTMUX_APP_PATH`.** Every process belonging to this
/// exact running instance (portable build or install) is launched from
/// under that one directory — confirmed directly (not assumed): this
/// agent's own host process's `Process::exe()` path
/// (`...\agentmux-0.55.18+....-x64-portable\runtime\agentmux-0.55.18.exe`)
/// starts with `AGENTMUX_APP_PATH`
/// (`...\agentmux-0.55.18+....-x64-portable`) exactly. This is more precise
/// than matching on version number alone — it correctly tells apart two
/// separate instances that happen to share a version (e.g. two portable
/// builds of the same release), which a version-string match would
/// conflate.
///
/// **Fallback signal: bounded process-ancestor walk.** Kept as a second,
/// additive layer (union, not replacement) for the case `AGENTMUX_APP_PATH`
/// isn't set (confirmed present for `AGENTMUX_RUNTIME_MODE=portable`; not
/// separately confirmed for `task dev` instances). **This alone is not
/// reliable** — verified directly by testing: Windows recycles PIDs and a
/// process's recorded parent-PID can point at an already-exited process, so
/// `sys.process(stale_ppid)` returns `None` and the walk silently stops
/// short of the real ancestor chain. Confirmed this exact failure in
/// testing (the walk never reached this instance's own host process). Kept
/// only as defense-in-depth alongside the path-based signal, never alone.
///
/// Fails safe: an unresolvable `pid()` on a *candidate* window (checked by
/// the caller, `CaptureWindow`'s handler) is treated as "assume it's mine,
/// exclude it" — so a window this function's own signals can't positively
/// place is still excluded, not silently let through.
pub(crate) fn own_instance_pids() -> std::collections::HashSet<u32> {
    let sys = sysinfo::System::new_all();
    let mut result = std::collections::HashSet::new();

    if let Ok(app_path) = std::env::var("AGENTMUX_APP_PATH") {
        if !app_path.is_empty() {
            let app_path = std::path::Path::new(&app_path);
            for (pid, proc) in sys.processes() {
                if proc.exe().map(|e| e.starts_with(app_path)).unwrap_or(false) {
                    result.insert(pid.as_u32());
                }
            }
        }
    }

    let my_pid = sysinfo::Pid::from(std::process::id() as usize);
    let mut ancestors = vec![my_pid];
    let mut current = my_pid;
    for _ in 0..OWN_INSTANCE_ANCESTOR_HOPS {
        let Some(proc) = sys.process(current) else { break };
        let Some(parent) = proc.parent() else { break };
        ancestors.push(parent);
        current = parent;
    }
    result.extend(ancestors.iter().map(|p| p.as_u32()));
    for (pid, proc) in sys.processes() {
        let Some(ppid) = proc.parent() else { continue };
        if !ancestors.contains(&ppid) {
            continue;
        }
        if proc.name().to_string_lossy().to_lowercase().starts_with("agentmux") {
            result.insert(pid.as_u32());
        }
    }
    result
}

/// Capture tier for one target window — see
/// `docs/specs/SPEC_AGENT_UNRESTRICTED_CAPTURE_WITH_ACCOUNTABILITY_2026_08_30.md`
/// §3. Keyed on WHAT is being captured, never on who is asking: identity is
/// already proven upstream (`sign_ui_automation_auth`), so the open question
/// is what a proven identity may reach.
///
/// This replaces the previous binary `!is_self` exclusion. That rule came from
/// `SPEC_AGENT_UI_AUTOMATION_CLICK_SCREENSHOT_2026_08_18.md` §6, which itself
/// says cross-agent targeting *"if it's ever wanted at all"* should be a
/// distinct capability — i.e. it defaulted closed because no mechanism existed
/// to be selective, not because open had been judged wrong. The repo owner has
/// since directed that agents be able to capture anything. This enum is that
/// mechanism.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CaptureTier {
    /// T1 — another pane inside the caller's OWN instance. Its window hosts
    /// every agent's pane, which is exactly what the old rule blocked.
    SameInstance,
    /// T2 — a different AgentMux instance owned by the same OS user.
    OtherInstance,
    /// T3 — a window owned by a DIFFERENT OS user. The one tier that crosses a
    /// human rather than an agent boundary: that user never consented to this
    /// app's agents, and no in-app notification can reach them.
    OtherUser,
    /// T4 — a non-AgentMux window owned by the same OS user (their browser,
    /// their password manager). PR #2709 round 2 caught the original unscoped
    /// tool capturing a KeePass window; allowed now, but always audited.
    ForeignApp,
}

impl CaptureTier {
    /// Phase 1 `allow` defaults (spec §3). Only T3 is closed, and only because
    /// it is a human-to-human boundary — every agent-to-agent tier is open.
    pub(crate) fn allowed(self) -> bool {
        !matches!(self, CaptureTier::OtherUser)
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            CaptureTier::SameInstance => "T1-same-instance",
            CaptureTier::OtherInstance => "T2-other-instance",
            CaptureTier::OtherUser => "T3-other-user",
            CaptureTier::ForeignApp => "T4-foreign-app",
        }
    }
}

/// The OS user id owning the calling process, used to place a target window on
/// the near or far side of the T3 boundary.
///
/// Fails CLOSED, matching `own_instance_pids()`'s existing discipline: if this
/// returns `None` every window resolves to `OtherUser` and capture is denied.
/// An unresolvable owner is exactly the case where guessing "probably mine"
/// would be how a cross-user capture slips through.
pub(crate) fn current_user_id(sys: &sysinfo::System) -> Option<String> {
    let me = sysinfo::get_current_pid().ok()?;
    sys.process(me)
        .and_then(|p| p.user_id())
        .map(|u| u.to_string())
}

/// A completed capture. Richer than the bare message string it replaces so
/// `audit_log_capture_window` can record WHAT was captured and at which tier,
/// rather than only the caller's query and a success/failure flag.
#[derive(Debug)]
pub(crate) struct CaptureOutcome {
    /// Human-facing tool result, returned to the caller verbatim.
    pub(crate) message: String,
    pub(crate) tier: CaptureTier,
    /// Resolved target — `pid=N title="..."`.
    pub(crate) target: String,
    /// Hex SHA-256 of the saved PNG; `None` if hashing failed (best-effort).
    pub(crate) image_sha256: Option<String>,
}

/// One top-level window visible to this process, with the metadata both
/// `DiscoverWindows` and `CaptureWindow` need. Shared via
/// `enumerate_agentmux_windows()` so the two tools can't drift on what counts
/// as "AgentMux's own window", "the calling agent's own instance", or which
/// tier a target falls in.
pub(crate) struct AgentMuxWindowInfo {
    pub(crate) window: xcap::Window,
    pub(crate) pid: u32,
    pub(crate) title: String,
    pub(crate) exe_path: String,
    pub(crate) is_self: bool,
    /// Whether the owning process is an AgentMux process at all — T4 targets
    /// are enumerated now, so this is no longer implied by presence in the list.
    pub(crate) is_agentmux: bool,
    pub(crate) tier: CaptureTier,
}

/// Enumerate every top-level OS window, AgentMux-owned or not, each tagged
/// with the `CaptureTier` that decides whether it may be captured.
///
/// `is_agentmux` is matched via `app_name()`, not `title()` — AgentMux's own
/// process names are version-stamped, e.g. `agentmux-0.55.18`, not a single
/// fixed string, but they all share the `agentmux` prefix (reagent P1, PR
/// #2709 round 2). That flag no longer decides *inclusion* — non-AgentMux
/// windows are T4 capture targets — but it still gates disclosure: they stay
/// out of `DiscoverWindows`' default listing and their titles are withheld
/// from candidate/miss lists (`candidate_label`), which is what keeps round
/// 2's KeePass-title leak closed now that they're enumerated at all.
///
/// `is_self` marks windows belonging to the calling agent's OWN instance
/// (`own_instance_pids()`).
///
/// **Historical note (reagent P2 on PR #2845):** this comment previously said
/// `is_self` windows were "hard-excluded downstream by `capture_window_impl`
/// … the actual isolation boundary, not a nicety" (reagent P0, PR #2709
/// round 3). That is no longer true and the reasoning has been superseded, not
/// merely relaxed: the boundary it protected —
/// `SPEC_AGENT_UI_AUTOMATION_CLICK_SCREENSHOT_2026_08_18.md` §6's own-pane-only
/// default — was an unratified recommendation that defaulted closed because no
/// mechanism existed to be selective. `is_self` now resolves to
/// `CaptureTier::SameInstance`, which is allowed. See
/// `SPEC_AGENT_UNRESTRICTED_CAPTURE_WITH_ACCOUNTABILITY_2026_08_30.md`.
pub(crate) fn enumerate_agentmux_windows() -> Result<Vec<AgentMuxWindowInfo>> {
    let windows =
        xcap::Window::all().map_err(|e| anyhow::anyhow!("failed to enumerate windows: {e}"))?;
    let own_pids = own_instance_pids();
    let sys = sysinfo::System::new_all();
    let me = current_user_id(&sys);
    let mut out = Vec::new();
    for window in windows {
        let is_agentmux = window
            .app_name()
            .map(|a| a.to_lowercase().starts_with("agentmux"))
            .unwrap_or(false);
        // Non-AgentMux windows are no longer skipped — they are T4 targets.
        // They stay out of `DiscoverWindows`' default listing (see its
        // `include_foreign` arg) so ordinary discovery doesn't disclose the
        // titles of a user's unrelated applications as a side effect.
        let pid = window.pid().unwrap_or(0);
        // Fails safe: an unresolvable pid is treated as "assume it's mine"
        // rather than silently dropped, so `DiscoverWindows` still surfaces
        // that the window exists instead of hiding it.
        //
        // That promise is real again as of reagentx P2 on PR #2845. Briefly it
        // wasn't: an unresolvable OWNER resolves the window to T3, and
        // `DiscoverWindows` was dropping non-allowed tiers outright, so such a
        // window vanished from the listing even with `include_self`. It is now
        // listed with `title`/`exe_path` redacted, which keeps the disclosure
        // closed without reintroducing the hiding this comment warns against.
        let is_self = pid == 0 || own_pids.contains(&pid);
        let proc = sys.process(sysinfo::Pid::from(pid as usize));
        let title = window.title().unwrap_or_default();
        let exe_path = proc
            .and_then(|p| p.exe())
            .map(|p| p.display().to_string())
            .unwrap_or_default();

        // Tier resolution. The OS-user check runs FIRST and outranks
        // everything else: a window owned by another human is T3 no matter
        // what process owns it, including another AgentMux instance of theirs.
        let owner = proc.and_then(|p| p.user_id()).map(|u| u.to_string());
        let tier = match (&me, &owner) {
            // Both known and different — the human boundary.
            (Some(mine), Some(theirs)) if mine != theirs => CaptureTier::OtherUser,
            // Either side unresolvable — fail closed, same discipline as
            // `is_self` above. Guessing "probably mine" here is precisely how
            // a cross-user capture would slip through.
            (None, _) | (_, None) => CaptureTier::OtherUser,
            _ if is_self => CaptureTier::SameInstance,
            _ if is_agentmux => CaptureTier::OtherInstance,
            _ => CaptureTier::ForeignApp,
        };

        out.push(AgentMuxWindowInfo {
            window,
            pid,
            title,
            exe_path,
            is_self,
            is_agentmux,
            tier,
        });
    }
    Ok(out)
}

/// Cheap heuristic for "this capture is probably a blank/unpainted frame,
/// not a real render" — sample ~200 evenly-spaced pixels and check they're
/// all within a small tolerance of the first one. Not real image analysis
/// (a legitimately solid-color themed window would also trip this) —
/// deliberately cheap and approximate, used only to decide whether a short
/// bounded retry is worth attempting, and to set `likely_unrendered` as a
/// hint, never as a hard failure.
pub(crate) fn looks_unrendered(img: &image::RgbaImage) -> bool {
    let pixels = img.as_raw();
    if pixels.len() < 4 {
        return true;
    }
    let (r0, g0, b0) = (pixels[0], pixels[1], pixels[2]);
    let pixel_count = pixels.len() / 4;
    let step = (pixel_count / 200).max(1);
    let mut sampled = 0usize;
    for i in (0..pixel_count).step_by(step) {
        let idx = i * 4;
        sampled += 1;
        if pixels[idx].abs_diff(r0) > 8 || pixels[idx + 1].abs_diff(g0) > 8 || pixels[idx + 2].abs_diff(b0) > 8 {
            return false;
        }
    }
    sampled > 0
}

/// Label for a window in the AUDIT TRAIL.
///
/// Fuller than `candidate_label` where it can be — reagentx P2 on PR #2845
/// correctly spotted that reusing the caller-facing label lost a T4 window's
/// title on a post-resolution failure, giving foreign-app failures less audit
/// detail than AgentMux ones for no reason.
///
/// But NOT unconditionally full, because that finding's premise — "the audit
/// is server-side only" — does not hold here. The trail is an NDJSON file in
/// this instance's own data dir, and agents have shell access; it was read
/// with `tail` during this PR's own verification. It is therefore not a
/// confidential sink.
///
/// So audit detail follows the ALLOW decision, not AgentMux-ness:
///   - allowed tier → full title. The agent could capture the window and read
///     the title off the pixels anyway, so recording it discloses nothing new.
///   - withheld tier → pid and tier only. Recording a T3 title would hand the
///     agent, via a file it can read, exactly the content the tier denied it —
///     reopening through the log the same oracle closed in the title-miss path.
pub(crate) fn audit_target_label(w: &AgentMuxWindowInfo) -> String {
    if w.tier.allowed() {
        format!("pid={} title={:?}", w.pid, w.title)
    } else {
        format!("pid={} {} <title withheld>", w.pid, w.tier.label())
    }
}

/// Label for a window in a CALLER-FACING disambiguation/candidate list.
///
/// Stricter than `audit_target_label` above, because the audiences differ: this
/// one is returned to the agent, so it withholds the TITLE of every
/// non-AgentMux window — reagent P1 on PR #2845. Foreign windows became
/// capturable with the tier model, so any list built from the capturable set
/// would otherwise disclose a user's unrelated app titles (their browser,
/// their password manager) as a side effect of a miss or an ambiguous match,
/// bypassing `DiscoverWindows`' own `include_foreign` opt-in. The pid alone
/// disambiguates, and is what the caller needs anyway.
pub(crate) fn candidate_label(w: &AgentMuxWindowInfo) -> String {
    if w.is_agentmux {
        format!("pid={} title={:?}", w.pid, w.title)
    } else {
        format!("pid={} <non-AgentMux window; pass include_foreign to DiscoverWindows to identify it>", w.pid)
    }
}

/// The actual `CaptureWindow` logic — window enumeration, own-instance and
/// third-party-app exclusion, targeting (by pid or by title), capture, and
/// save. Extracted to its own function (rather than living inline in the
/// `"CaptureWindow" =>` match arm) so its `Result` can be captured once and
/// unconditionally audit-logged by the caller before propagating — see
/// `audit_log_capture_window`.
///
/// `index: None` with more than one `title_contains` match is an error
/// listing every candidate, NOT a silent pick of match 0 — see
/// docs/reports/REPORT_AGENT_SCREENSHOT_WINDOW_CONTROL_BLOCKERS_2026_08_24.md
/// §1 for the real incident (an ambiguous match silently captured an
/// unrelated, sensitive window) this specifically fixes. `index: Some(i)`
/// is still honored directly against whatever matched, unchanged from the
/// original behavior, for callers who already disambiguate explicitly.
pub(crate) fn capture_window_impl(
    title_contains: Option<&str>,
    index: Option<usize>,
    pid: Option<u32>,
    // Filled the moment a target is RESOLVED, so a failure after that point —
    // a denied T3 pid, a failed capture_image, a failed PNG write — is still
    // audited with its tier and target instead of nulls (codex P2 on PR #2845).
    // Without it the log cannot tell "no such window" apart from "blocked
    // cross-user attempt", which is the entry a reviewer most wants to find.
    resolved: &mut Option<(CaptureTier, String)>,
) -> Result<CaptureOutcome> {
    let all = enumerate_agentmux_windows()?;
    // Tier gate replaces the old `!w.is_self` exclusion — see `CaptureTier`.
    // Only T3 (a different OS user's window) is withheld; every agent-to-agent
    // tier, including this instance's own window, is reachable.
    let foreign: Vec<&AgentMuxWindowInfo> = all.iter().filter(|w| w.tier.allowed()).collect();

    let target: &AgentMuxWindowInfo = if let Some(target_pid) = pid {
        // A single host process can own multiple top-level windows sharing
        // the same pid (agentmux-cef/src/browser_pane/hwnd.rs:200-204,
        // agentmux-cef/src/commands/window/lifecycle.rs:410-426) — codex P1
        // on PR #2810: the original `.find()` here silently captured
        // whichever matching window enumerated first, which could be a
        // pool/sub-window rather than the one the caller meant, reintroducing
        // the wrong-window capture this PR exists to prevent. Same
        // ambiguity-rejection shape as the title_contains branch below:
        // `index` disambiguates, an unqualified ambiguous match does not.
        let matches: Vec<&AgentMuxWindowInfo> = foreign
            .iter()
            .filter(|w| w.pid == target_pid)
            .copied()
            .collect();
        if matches.is_empty() {
            // `foreign` is already tier-filtered, so a withheld T3 window is
            // absent from it and would otherwise be indistinguishable from a
            // pid that doesn't exist — reported identically to the caller AND
            // audited as `tier: null`. Look it up in the UNFILTERED set to
            // separate "withheld" from "absent" (codex P2 / reagentx P2).
            if let Some(blocked) = all.iter().find(|w| w.pid == target_pid) {
                *resolved = Some((blocked.tier, audit_target_label(blocked)));
                anyhow::bail!(
                    "window pid={target_pid} is {} — capture is withheld for windows owned \
                     by a different OS user; every other tier is available",
                    blocked.tier.label()
                );
            }
            anyhow::bail!(
                "no window found for pid {target_pid} — call DiscoverWindows to see \
                 current candidates"
            );
        }
        match index {
            Some(i) => *matches.get(i).ok_or_else(|| {
                anyhow::anyhow!(
                    "index {i} out of range — only {} window(s) matched pid {target_pid}",
                    matches.len()
                )
            })?,
            None if matches.len() == 1 => matches[0],
            None => {
                let candidates: Vec<String> = matches
                    .iter()
                    .map(|w| candidate_label(w))
                    .collect();
                anyhow::bail!(
                    "{} windows matched pid {target_pid} — a single process can own \
                     multiple top-level windows; pass an explicit index (0-based) \
                     alongside pid to disambiguate. Candidates: {}",
                    matches.len(),
                    candidates.join("; ")
                );
            }
        }
    } else {
        let title_contains = title_contains
            .ok_or_else(|| anyhow::anyhow!("must provide either pid or title_contains"))?;
        let needle = title_contains.to_lowercase();
        let matches: Vec<&AgentMuxWindowInfo> = foreign
            .iter()
            .filter(|w| w.title.to_lowercase().contains(&needle))
            .copied()
            .collect();

        if matches.is_empty() {
            // A title matching ONLY a withheld window is recorded for the
            // AUDIT — and deliberately changes nothing the caller sees.
            //
            // reagentx P1 on PR #2845: an earlier version bailed here with a
            // distinguishable "exists but is T3" message. Withholding the
            // literal title wasn't enough — differing *responses* are an
            // existence oracle. An agent can probe substrings (binary-search
            // over characters) and reconstruct a cross-user window's title
            // without ever being granted capture, defeating the very boundary
            // this tier exists to hold. Introduced while fixing the audit gap
            // below it: making withheld-vs-absent legible to the auditor made
            // it legible to the attacker too.
            //
            // The split that resolves it: the AUDIT is server-side and may
            // know; the CALLER may not. So this records `resolved` and then
            // falls through to the identical generic-miss message below —
            // structurally the same code path and the same bytes either way,
            // rather than two branches a future edit could let drift apart.
            //
            // The pid branch above is deliberately NOT symmetric: it does name
            // the tier. A caller must already hold the pid to ask, `Discover
            // Windows` never hands out T3 pids, and process ownership is
            // already enumerable by anything with shell access — so the pid's
            // existence is not a secret this tool is keeping, while a window
            // TITLE is exactly the content it is.
            if let Some(blocked) = all
                .iter()
                .find(|w| !w.tier.allowed() && w.title.to_lowercase().contains(&needle))
            {
                *resolved = Some((blocked.tier, audit_target_label(blocked)));
            }
            // Only lists AGENTMUX windows' titles, never every window on the
            // desktop — reagent P2 (PR #2709 round 2): the original version
            // dumped every visible window's title on any miss, which let a
            // caller enumerate arbitrary window titles (confirmed in testing:
            // it leaked a password manager's document title) with no real
            // match required at all.
            //
            // The `is_agentmux` filter is load-bearing again as of the tier
            // model — reagent P1 on PR #2845. `foreign` now includes T4
            // non-AgentMux windows (they became capturable), so listing it
            // wholesale would have re-opened exactly that leak, and would have
            // bypassed `DiscoverWindows`' own `include_foreign` opt-in in the
            // process: a caller could enumerate the user's desktop by
            // deliberately missing.
            let titles: Vec<&str> = foreign
                .iter()
                .filter(|w| w.is_agentmux)
                .map(|w| w.title.as_str())
                .filter(|t| !t.is_empty())
                .collect();
            if titles.is_empty() {
                anyhow::bail!(
                    "no AgentMux window title contains {title_contains:?} — \
                     no other AgentMux windows are currently open"
                );
            }
            anyhow::bail!(
                "no AgentMux window title contains {title_contains:?}. \
                 Open AgentMux window titles: {}",
                titles.join(", ")
            );
        }

        match index {
            Some(i) => *matches.get(i).ok_or_else(|| {
                anyhow::anyhow!(
                    "index {i} out of range — only {} window(s) matched {title_contains:?}",
                    matches.len()
                )
            })?,
            None if matches.len() == 1 => matches[0],
            None => {
                // The fix for blocker #1 in the report: an ambiguous
                // match with no explicit index used to silently capture
                // index 0 despite this tool's own docstring claiming it
                // would list candidates instead. It now actually does.
                let candidates: Vec<String> = matches
                    .iter()
                    .map(|w| candidate_label(w))
                    .collect();
                anyhow::bail!(
                    "{} windows matched {title_contains:?} — pass an explicit index (0-based), \
                     or better, one of these pids directly (preferred: stable, unlike title). \
                     Candidates: {}",
                    matches.len(),
                    candidates.join("; ")
                );
            }
        }
    };

    *resolved = Some((target.tier, audit_target_label(target)));

    let title = target.title.clone();

    // Short bounded retry for a freshly-created window that hasn't
    // painted its first real frame yet — see looks_unrendered()'s doc
    // comment. std::thread::sleep (not tokio::time::sleep): this fn is
    // plain sync code called from the async dispatch path, and the
    // bounded ~800ms worst case is a deliberate, request-scoped wait
    // directly serving this call, not background work stealing the
    // runtime from anything else.
    let mut image = target
        .window
        .capture_image()
        .map_err(|e| anyhow::anyhow!("capture failed: {e}"))?;
    let mut likely_unrendered = looks_unrendered(&image);
    let mut attempts = 1;
    while likely_unrendered && attempts < 3 {
        std::thread::sleep(Duration::from_millis(400));
        image = target
            .window
            .capture_image()
            .map_err(|e| anyhow::anyhow!("capture failed: {e}"))?;
        likely_unrendered = looks_unrendered(&image);
        attempts += 1;
    }

    let dir = capture_window_dir();
    std::fs::create_dir_all(&dir)
        .map_err(|e| anyhow::anyhow!("failed to create capture dir {}: {e}", dir.display()))?;
    let path = dir.join(format!("{}.png", uuid::Uuid::new_v4()));
    image
        .save(&path)
        .map_err(|e| anyhow::anyhow!("failed to save capture to {}: {e}", path.display()))?;
    // Unbounded growth over repeated/looped calls otherwise — same bug
    // class as UIScreenshot's own screenshot dir (reagent P2, PR #2662)
    // and reagent's own review of this PR (round 1) pointing out that
    // fix wasn't reused here. Same fix, same shape: best-effort, on the
    // write path.
    prune_old_captures(&dir);

    // codex P2 on PR #2810: no process start time is collected or checked
    // anywhere, so claiming "this window was created recently" was
    // unsupported and misleading for a mature window that just happens to
    // render a near-uniform frame (looks_unrendered's own doc comment
    // already acknowledges that legitimate case). State only what was
    // actually observed.
    let hint = if likely_unrendered {
        " (likely_unrendered: true — the captured frame looks solid or \
           near-solid-color even after retrying, which can mean the window \
           hasn't painted its first real frame yet, or that it genuinely \
           looks like this; consider calling CaptureWindow again shortly if \
           that's unexpected)"
    } else {
        ""
    };
    Ok(CaptureOutcome {
        message: format!(
            "Captured window {title:?} to {}{hint} — use Read on that path to view it yourself, or OpenMedia to show it to the user.",
            path.display()
        ),
        tier: target.tier,
        // The resolved target, not the query string — an audit reviewer needs
        // to know WHAT was captured, which a `title_contains` substring does
        // not tell them.
        target: format!("pid={} title={:?}", target.pid, target.title),
        image_sha256: sha256_file(&path),
    })
}

/// Hex SHA-256 of a captured PNG, recorded in the audit trail so a leaked
/// screenshot can be traced back to the call that produced it (spec §6).
/// Best-effort — a hashing failure yields `None` and must never fail the
/// capture, matching `audit_log_capture_window`'s own "never break the tool"
/// discipline.
pub(crate) fn sha256_file(path: &std::path::Path) -> Option<String> {
    use sha2::{Digest, Sha256};
    let bytes = std::fs::read(path).ok()?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    Some(format!("{:x}", hasher.finalize()))
}

/// Audit trail for `CaptureWindow` — reagent P1 (PR #2709 round 4): the
/// residual risk after rounds 2-3's scoping (a different AgentMux
/// instance's window, which can belong to a different OS user on a shared
/// machine) is a disclosure-across-a-human-boundary question, not a
/// technical one a capability flag alone would fully answer — a real
/// per-agent opt-in gate is tracked as a separate follow-up (no existing
/// settings/enforcement mechanism to build it on today). Logging every
/// call — who, what was requested, what happened — is the honest,
/// shippable Phase-1 answer: best-effort (a logging failure must never
/// break the tool itself), append-only NDJSON inside `capture_window_dir()`
/// itself — `prune_old_captures`'s PNG-only extension filter already
/// leaves a `.log` file in that same directory untouched, so this doesn't
/// need (or want) its own separate directory alongside it.
pub(crate) fn audit_log_capture_window(
    query_desc: &str,
    outcome: &Result<CaptureOutcome>,
    // Resolved target/tier for the FAILURE paths — on success the outcome
    // carries its own. See capture_window_impl's `resolved` param.
    resolved: &Option<(CaptureTier, String)>,
) {
    let entry = serde_json::json!({
        "timestamp": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
        "agent_id": agent_slug().unwrap_or_else(|_| "unknown".to_string()),
        "tool": "CaptureWindow",
        // "pid=N" or "title_contains=\"...\"" — whichever targeting mode
        // the caller used (SPEC_AGENT_APP_API_WINDOW_CONTROL_ROBUSTNESS_2026_08_24.md
        // added pid-based targeting as an alternative to title matching).
        "query": query_desc,
        // The RESOLVED target and its tier, not just the query string — a
        // reviewer needs to know what was actually captured, which a
        // `title_contains` substring doesn't say. Absent on failure, since
        // nothing was resolved (spec §6).
        "tier": outcome
            .as_ref()
            .ok()
            .map(|c| c.tier.label())
            .or_else(|| resolved.as_ref().map(|(t, _)| t.label())),
        "target": outcome
            .as_ref()
            .ok()
            .map(|c| c.target.clone())
            .or_else(|| resolved.as_ref().map(|(_, t)| t.clone())),
        // Traces a leaked screenshot back to the call that produced it.
        "image_sha256": outcome.as_ref().ok().and_then(|c| c.image_sha256.clone()),
        // Phase 1 never redacts — recorded explicitly so an unredacted T2/T4
        // capture is distinguishable at review time once Phase 3 lands, rather
        // than the field's absence being ambiguous between "no" and "old entry".
        "redacted": false,
        "outcome": match outcome {
            Ok(c) => serde_json::json!({"result": "success", "detail": c.message}),
            Err(e) => serde_json::json!({"result": "error", "detail": e.to_string()}),
        },
    });
    append_window_audit_log_entry(&entry);
}

/// One `DiscoverWindows` entry.
///
/// A withheld (T3) window is LISTED but redacted, not omitted — reagentx P2 on
/// PR #2845 caught two contradictions the omit-entirely approach created:
///   - the `is_self` fail-safe promised an unresolvable-pid window would
///     "still surface ... instead of hiding it", which an unconditional drop
///     broke (an unresolvable owner resolves to T3)
///   - this tool advertises `capturable` so a caller can "see why a window is
///     out of reach before trying", unreachable if every listed entry is
///     capturable by construction
///
/// Redacting satisfies both while keeping the disclosure closed: `title` and
/// `exe_path` (which embeds the owning OS username) are exactly what must not
/// cross the human boundary, and they are the only fields dropped. A pid and
/// "someone else owns this" are already available to anything with shell
/// access, so surfacing them costs nothing — and it makes a wholesale
/// user-id-resolution failure diagnosable instead of silently returning an
/// empty list.
pub(crate) fn window_listing_entry(w: &AgentMuxWindowInfo) -> Value {
    if w.tier.allowed() {
        json!({
            "pid": w.pid,
            "title": w.title,
            "exe_path": w.exe_path,
            "is_self": w.is_self,
            "is_agentmux": w.is_agentmux,
            "tier": w.tier.label(),
            "capturable": true,
        })
    } else {
        json!({
            "pid": w.pid,
            "title": null,
            "exe_path": null,
            "is_self": w.is_self,
            "is_agentmux": w.is_agentmux,
            "tier": w.tier.label(),
            "capturable": false,
            "withheld_reason": "owned by a different OS user; title and exe_path withheld",
        })
    }
}

/// Audit trail for `DiscoverWindows` — reagent P1 on PR #2810: this tool
/// discloses `exe_path` (a full filesystem path, typically embedding the OS
/// username) for windows belonging to OTHER AgentMux instances/users on a
/// shared machine — the exact same disclosure-across-a-human-boundary risk
/// `audit_log_capture_window` above already exists to log, but this tool
/// shipped with none. Same shape, same file (a single window-related audit
/// trail is easier to review than two): best-effort, never blocks the
/// tool's own result.
pub(crate) fn audit_log_discover_windows(include_self: bool, include_foreign: bool, windows: &[Value]) {
    let entry = serde_json::json!({
        "timestamp": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
        "agent_id": agent_slug().unwrap_or_else(|_| "unknown".to_string()),
        "tool": "DiscoverWindows",
        // Both flags — reagentx P2 on PR #2845. `include_foreign` is the one
        // that actually triggers this trail's reason for existing: it
        // discloses non-AgentMux window titles and exe_paths. Recording only
        // `include_self` left a reviewer scanning query strings unable to tell
        // whether foreign disclosure was even requested, inferable only by
        // picking through each entry's `is_agentmux` tag.
        "query": format!("include_self={include_self} include_foreign={include_foreign}"),
        "outcome": serde_json::json!({
            "result": "success",
            "window_count": windows.len(),
            "windows": windows,
        }),
    });
    append_window_audit_log_entry(&entry);
}

/// Shared append-only NDJSON writer for both window-tool audit trails above
/// — best-effort (a logging failure must never break the tool itself),
/// inside `capture_window_dir()` itself since `prune_old_captures`'s
/// PNG-only extension filter already leaves a `.log` file in that same
/// directory untouched, so this doesn't need (or want) its own directory.
pub(crate) fn append_window_audit_log_entry(entry: &Value) {
    let dir = capture_window_dir();
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let line = format!("{entry}\n");
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("capture-window-audit.log"))
    {
        use std::io::Write;
        let _ = f.write_all(line.as_bytes());
    }
}
