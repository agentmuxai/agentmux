# SPEC: Linux Sandbox — Recover From AppArmor's Unprivileged-Userns Restriction

**Status:** Draft — spec only, no implementation yet
**Date:** 2026-08-23
**Scope:** `agentmux-cef` (Linux-only code paths), `scripts/build-appimage-linux.sh`
(new bundled helper + policy template), `assets/linux/` (new files).

---

## 1. Problem

Reported by the repo owner: on a real Ubuntu box, the `v0.55.21` AppImage
downloaded from GitHub Releases doesn't launch at all — no window, no visible
error — unless run with `--no-sandbox` (or the existing
`AGENTMUX_UNSAFE_NOSANDBOX=1` escape hatch). "This worked in the past."

### 1.1 Root cause (confirmed, not a regression in this repo)

`SPEC_CEF_SANDBOX_2026_06_20.md` §4.2 made a deliberate, reasonable choice at
the time: Linux uses Chromium's **kernel user-namespace sandbox**
(`--disable-setuid-sandbox`) rather than a root-owned SUID `chrome-sandbox`
binary — "no setuid helper... is needed on kernels ≥3.8." That's still true
*in general*. What's changed underneath it: Ubuntu backported an AppArmor
restriction on **unprivileged user-namespace creation** (originally landed in
23.10, later security-patched into 22.04/20.04 LTS too, ~early 2024) that
blocks exactly this sandbox path for any Chromium/Electron/CEF-based
application, system-wide, unless the specific binary has an AppArmor
exception. This is not AgentMux-specific — it broke Chrome itself, VS Code,
Discord, Slack, and others industry-wide around the same time. A box that
picked up this policy via `unattended-upgrades` between "worked" and "doesn't
work" explains the report exactly, with zero code change required on our end.

Confirmed independently this session:
- The downloaded `v0.55.21` AppImage's internal structure (icon, `.desktop`,
  `AppRun`) is completely correct — not a packaging defect.
- `chrome-sandbox` is copied into the AppImage
  (`scripts/build-appimage-linux.sh:150-158`) but is **never chmod'd, chown'd,
  or made setuid anywhere** in the build — confirmed dead weight, consistent
  with `docs/cef-build/build-patched-libcef.md:224-228` ("chrome-sandbox is
  intentionally omitted... ships fine without it").
- AppImage is the **only** Linux distribution format that actually exists
  today. `docs/linux.md` and `SPEC_LINUX_DOCS_UPDATE_2026_06_06.md` both claim
  a `.deb` is "produced by CI builder" — that claim is stale: the referenced
  builder repo was deleted 2026-07-18, and even before deletion never had a
  working Linux build. This matters structurally: a `.deb`'s root `postinst`
  script is exactly how Chrome's own `.deb` solves this same problem at
  install time — AppImage has no equivalent hook, because it has no install
  step at all.

### 1.2 Why "transparent AND actually sandboxed" is a real tradeoff, not a UX choice

The repo owner's ask: "it was working before... we need the sandbox to work"
— not a silent `--no-sandbox` fallback, and not a manual flag the user has to
discover. Taken literally, this requires the kernel-namespace sandbox to
*actually function* on an affected Ubuntu box, transparently.

That is only possible via one of:
1. An AppArmor profile exception for AgentMux's binary — requires **root**
   to write to `/etc/apparmor.d/` and reload the AppArmor cache.
2. Reviving the classic SUID `chrome-sandbox` path — requires **root** to
   `chown root:root` + `chmod 4755` the binary.

There is no way to make an *unprivileged* AppImage produce a working
namespace sandbox on a system that has deliberately restricted unprivileged
namespace creation — that restriction is the whole point of the policy. Some
one-time privileged action is unavoidable if the goal is a real sandbox
rather than `--no-sandbox`. The honest design question isn't "how do we avoid
that" — it's "how do we make that one-time action as close to transparent as
possible, and make it survive future updates instead of repeating forever."

### 1.3 Why AppArmor (option 1), not reviving SUID chrome-sandbox (option 2)

`AppRun`'s extract-once-cache (`scripts/linux-apprun.sh`, Phase 2 of
`docs/specs/linux-appimage-cold-launch-tax-2026-05-08.md`) extracts a **fresh,
version-scoped copy** of the AppImage's contents to
`$HOME/.local/share/agentmux/extracted/<VERSION>/` on first run of each new
version. Every extraction is owned by the extracting user, non-setuid, by
construction — there is no way for a one-time `chown root + chmod 4755` on
`chrome-sandbox` to survive an AgentMux update, since the update produces an
entirely new file at an entirely new path. Option 2 would require re-running
the privileged fix on **every single AgentMux upgrade** — worse than useless
for "transparent," since it reintroduces friction on a cadence the user
doesn't control.

AppArmor profiles support path globbing (`/home/*/.local/share/agentmux/
extracted/*/usr/bin/agentmux-cef`). A profile written with a wildcard on the
version segment covers every past and future extracted version with **one**
installation, ever. This is the deciding factor — §5 designs around it.

---

## 2. Goals

- On an affected Ubuntu/Debian box, detect the specific userns-restriction
  failure (not generic CEF init failures — those already have a Windows
  precedent to mirror, see §4) and offer a one-time, clearly-explained,
  user-consented privileged fix that **installs an AppArmor exception**
  scoped to survive future AgentMux updates.
- Never fail silently. Whatever happens — fixed, declined, or `pkexec`
  unavailable — the user sees a real, native dialog explaining what
  happened and why. This closes the original bug report's actual proximate
  symptom ("double-click does nothing") regardless of which path below is
  taken.
- Never auto-disable the sandbox without explicit, visible user action. If
  the user declines the fix, the app may proceed with `--no-sandbox` for
  that session only if they explicitly choose to, with a persistent,
  visible (not dismiss-and-forget) indicator that it's running unsandboxed.
- Scope narrowly: only distros with this specific AppArmor restriction
  should ever see this flow. Distros where the namespace sandbox already
  works (most non-Ubuntu-family Linux, and Ubuntu boxes without the
  restriction) must see **zero behavior change** — no spurious prompts.

## 3. Non-goals

- Reviving SUID `chrome-sandbox` (§1.3 — rejected, doesn't survive updates).
- Building a `.deb`/other installable package format. Real, and would solve
  this more conventionally (root postinst), but is a materially bigger
  undertaking (packaging pipeline, signing, apt repo hosting) than fixing
  the AppImage in place. Worth a separate spec if the project decides to
  pursue a "real" Linux package later — flagged as OQ4, not attempted here.
- Fixing the underlying Ubuntu policy itself (out of our control) or telling
  users to run `sysctl kernel.apparmor_restrict_unprivileged_userns=0`
  system-wide — that's a global security downgrade affecting every app on
  the box, not just AgentMux; an AppArmor exception scoped to our own binary
  path is the narrower, more defensible ask of the user's trust.
- Non-Ubuntu/Debian distros' sandbox story — unaffected by this spec, no
  changes needed there.

---

## 4. Detection — extend the existing Windows precedent, don't invent a new pattern

`agentmux-cef/src/lib.rs` already has the exact shape of fix needed, just for
Windows: when `CefInitialize` returns a non-success `exit_code`, the `_ =>`
arm (`lib.rs` ~945-980) logs to `cef-debug.log` and — **because the web
frontend can't render when the host itself failed to start** — shows a
native `MessageBoxW` dialog rather than vanishing silently, with a comment
explicitly naming this as the fix for "a silent splash-then-exit."

That comment describes precisely the bug reported here, just on the other
platform. The Linux fix is a sibling branch in the same `_ =>` arm, not new
architecture:

```rust
#[cfg(target_os = "linux")]
{
    // Distinguish "namespace sandbox blocked" from other CEF init failures
    // (needs empirical verification — see rollout §8 step 1 — this is the
    // one piece of this spec that isn't grounded in a confirmed signature
    // yet).
    if linux_sandbox::is_userns_restricted_failure(exit_code, &cef_log_path) {
        linux_sandbox::show_userns_fix_dialog(); // §5
    } else {
        linux_sandbox::show_generic_startup_error_dialog(exit_code, &cef_log_path);
    }
}
```

**Open implementation question, called out honestly:** I have not yet
empirically captured what CEF's exit code / `cef-debug.log` content actually
look like when Chromium's namespace-sandbox setup is blocked by AppArmor vs.
some other init failure (missing GPU, corrupt bundle, etc.). Rollout step 1
(§8) is dedicated to capturing this signature for real, in a container with
the restriction actively enabled, before writing the detection logic — do not
guess at a specific exit code here.

A second, cheaper detection layer worth adding regardless of the exact
signature: **probe proactively, before ever calling `CefInitialize`.** Linux
lets an unprivileged process attempt `unshare(CLONE_NEWUSER)` directly; if it
fails with `EPERM` while `/proc/sys/kernel/apparmor_restrict_unprivileged_userns`
reads `1` (or the file doesn't exist but the syscall still fails), we know
authoritatively — cheaply, without waiting for a full CEF init attempt — that
this exact restriction is the cause. This avoids the exit-code-signature
guessing entirely for the common case and only needs the exit-code path as a
fallback for less-common init failures. Recommend implementing the proactive
probe as the primary detection path.

## 5. The one-time fix — AppArmor profile via `pkexec`

### 5.1 Profile content

```
# /etc/apparmor.d/agentmux-userns (installed by AgentMux's setup helper)
abi <abi/4.0>,
include <tunables/global>

profile agentmux-userns /home/*/.local/share/agentmux/extracted/*/usr/bin/agentmux-cef flags=(unconfined) {
  userns,
}
```

The wildcard on the version segment (`extracted/*/`) is the piece that makes
this survive every future AgentMux update without reinstallation — see §1.3.
Also needs a second stanza (or a second profile) covering the **FUSE-mount
path** for the rarer case where extraction itself failed (disk full, `$HOME`
unwritable) and `AppRun` falls back to running straight from the mount —
lower priority since it's already a degraded path, but should be covered for
completeness (exact mount path pattern is `/tmp/.mount_AgentMu*/usr/bin/
agentmux-cef` — appimage's default FUSE mountpoint naming scheme, needs
confirming against the actual runtime AgentMux ships).

### 5.2 Helper script + `pkexec`

Ship a small bundled helper, `assets/linux/install-userns-apparmor-fix.sh`,
that:
1. Writes the profile above to `/etc/apparmor.d/agentmux-userns`.
2. Reloads it: `apparmor_parser -r /etc/apparmor.d/agentmux-userns`.
3. Exits 0 on success, non-zero with a clear stderr message otherwise (e.g.
   AppArmor itself not installed/active — some Ubuntu derivatives disable it).

Invoked as `pkexec /path/to/install-userns-apparmor-fix.sh` from the dialog's
"Fix it now" action. `pkexec` (PolicyKit) is present on essentially every
GNOME/KDE desktop Ubuntu ships by default — it's what produces the familiar
graphical password prompt, no custom privilege-escalation UI needs building.
If `pkexec` itself isn't found on `PATH` (minimal/non-graphical setups), fall
back to printing the exact `sudo` command for the user to run manually in a
terminal, in the same dialog — still transparent about what's needed and why,
never silently disabled.

### 5.3 Dialog UX (native, not web-frontend-dependent — §4's reasoning applies)

Same `zenity`/`kdialog` availability-probe pattern as any other
desktop-Linux-agnostic native dialog (check `PATH` for each, prefer whichever
matches the running desktop's toolkit if detectable, fall back to plain
stderr text as the last resort for a fully headless/minimal box):

> **AgentMux — sandbox blocked by system policy**
>
> Your system's security policy (AppArmor) is blocking the sandbox AgentMux
> uses to isolate its browser engine. This is a known Ubuntu policy change,
> not an AgentMux bug — see [link to a docs/linux.md section this spec adds].
>
> [Fix it now (one-time, needs your password)] [Continue without sandbox this
> time] [Cancel]

- **Fix it now** → `pkexec` helper (§5.2) → on success, relaunch AgentMux
  automatically (`exec` the same AppImage/extracted binary again) so the user
  doesn't have to manually re-launch.
- **Continue without sandbox this time** → sets `AGENTMUX_UNSAFE_NOSANDBOX=1`
  for this process only (not persisted) and proceeds — but the main window,
  once it comes up, shows a **persistent, non-dismissable-by-accident**
  indicator (status bar icon or similar, reusing whatever pattern
  `frontend/app/store/flash-notifications.ts` or an equivalent persistent
  (not auto-expiring) banner mechanism provides) that the session is running
  without the OS sandbox, with a link back to re-trigger "Fix it now" later
  without needing to hit the failure again.
- **Cancel** → exits cleanly, same as today's behavior, but now with a clear
  reason instead of silence.

---

## 6. Files to create / modify

| File | Action |
|---|---|
| `agentmux-cef/src/lib.rs` | **Modify** — Linux sibling branch in the existing CEF-init-failure `_ =>` arm (§4) |
| `agentmux-cef/src/linux_sandbox.rs` (new) | **Create** — proactive `unshare(CLONE_NEWUSER)` probe, dialog display, `pkexec` invocation, exit-code/log fallback detection |
| `assets/linux/install-userns-apparmor-fix.sh` (new) | **Create** — the `pkexec`-invoked helper (§5.2) |
| `assets/linux/agentmux-userns.apparmor` (new) | **Create** — the profile template (§5.1), read by the helper script rather than inlined, so it's one source of truth |
| `scripts/build-appimage-linux.sh` | **Modify** — bundle the new helper + profile template into the AppDir (mirrors how `install-linux-desktop.sh` is already bundled) |
| `docs/linux.md` | **Modify** — document the AppArmor issue, the fix dialog, and the manual `pkexec`/`sudo` command as a reference (the dialog links here) |
| `docs/specs/SPEC_CEF_SANDBOX_2026_06_20.md` | **Modify (small)** — add a forward-reference note in §4.2 pointing at this spec, so a future reader doesn't think the "no setuid helper needed" claim is still unconditionally true |

---

## 7. Alternatives considered

- **Silent fallback to `--no-sandbox` when detection fires.** Explicitly
  rejected — the repo owner's direct feedback was "we need the sandbox to
  work," not "make the failure invisible." A silent downgrade of a security
  boundary is a worse outcome than an explained one, even if it's more
  convenient short-term.
- **Just fix the silent failure (show an error, don't offer a real fix).**
  Considered as a smaller first step, but doesn't satisfy "we need the
  sandbox to work" — would ship a clearer dead end, not a resolution. If the
  full AppArmor-fix flow (§5) turns out to be too large a first PR, this is
  a reasonable **Phase 1** to land before Phase 2 adds the actual fix — see
  rollout §8.
- **Reviving SUID `chrome-sandbox`.** Rejected in §1.3 — doesn't survive
  AgentMux updates due to the extract-once-cache's version-scoped paths.
- **Ship a `.deb` with a proper root postinst.** The conventional, "correct"
  fix — genuinely worth doing eventually (OQ4) — but a materially larger
  project (build pipeline, signing infra, hosting) than patching the
  existing AppImage, and doesn't help users who specifically want the
  portable AppImage distribution anyway.
- **Tell users to run `sysctl kernel.apparmor_restrict_unprivileged_userns=0`
  system-wide.** Rejected as the *recommended* path (though worth mentioning
  as a manual alternative in docs) — it's a global security downgrade
  affecting every application on the machine, not scoped to AgentMux the way
  an AppArmor profile exception is.

---

## 8. Rollout plan

1. **Empirically capture the real failure signature first**, before writing
   any detection logic (§4's open question) — reproduce the AppArmor
   restriction in a container (`docker run --cap-drop=all` plus explicitly
   setting the restriction, or a real Ubuntu 22.04+ VM with the security
   update applied) and record: does `CefInitialize` return non-zero at all,
   or does the process abort/crash before reaching that check? What (if
   anything) lands in `cef-debug.log`? This determines whether the proactive
   `unshare()` probe (recommended, §4) is sufficient on its own or the
   exit-code path is also needed as a fallback.
2. **Phase 1 (smaller, ships first): stop failing silently.** Implement just
   the generic native-dialog-on-CEF-init-failure branch for Linux (mirroring
   the existing Windows `MessageBoxW` precedent, §4), without the AppArmor
   fix yet. This alone directly fixes the original bug report's proximate
   symptom and is independently valuable/lower-risk to ship first.
3. **Phase 2: the real fix.** Add the proactive `unshare()` probe, the
   AppArmor profile + helper script + `pkexec` flow (§5), and the persistent
   unsandboxed-session indicator.
4. **Test on real hardware/VMs across the matrix that matters**: an
   Ubuntu 22.04 box with the security update (restricted), an Ubuntu box
   without it (unrestricted — confirm zero behavior change), and a
   non-Ubuntu distro (e.g. Fedora — confirm zero behavior change, since
   this is an AppArmor-specific, not universal-Linux, problem).
5. **Confirm the AppArmor profile actually survives an AgentMux version
   bump** — install the fix under version A, upgrade to version B (new
   extraction path), confirm the sandbox still works without re-prompting.
   This is the entire point of the wildcard design (§1.3) — worth a
   dedicated verification step, not just trusting the glob syntax works as
   intended.

---

## 9. Open questions

| # | Question | Notes |
|---|---|---|
| OQ1 | Exact CEF exit-code/log signature for a userns-blocked failure — see §4/§8 step 1. | Blocking for Phase 2's exit-code fallback path; the proactive probe (§4) may make this moot for the common case. |
| OQ2 | Should "Continue without sandbox this time" persist across restarts (a settings toggle) rather than being purely session-scoped? | Leaning no — re-prompting each launch keeps the unsandboxed state visible/intentional rather than something a user forgets they agreed to once; revisit if this proves annoying in practice. |
| OQ3 | Confirm the exact FUSE default mountpoint naming AppImage's runtime uses on this project's pinned `appimagetool` version, for the second AppArmor profile stanza (§5.1). | Needed for full coverage of the (rarer) non-extracted-cache launch path. |
| OQ4 | Is a proper `.deb` (with root postinst) worth pursuing as a follow-up project, given it would solve this more conventionally and also solves other install-time needs (desktop integration, updates)? | Flagged, not answered here — separate spec if pursued. |
| OQ5 | Does KDE's Ubuntu variant (Kubuntu) or other AppArmor-shipping non-Ubuntu distros (some SUSE configs) have the same unprivileged-userns restriction, meaning the "Ubuntu/Debian-family" scoping in Goals should be "any AppArmor-enabled distro with this specific restriction" instead? | Worth confirming during rollout step 4's cross-distro testing rather than assuming Ubuntu-only. |
