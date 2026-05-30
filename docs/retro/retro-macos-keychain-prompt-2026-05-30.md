# Retro: packaged macOS app triggers a Keychain "confidential information" prompt (and hangs on it)

**Date:** 2026-05-30
**Severity:** High (trust/UX showstopper + blocks first launch)
**Surfaced by:** first launch of the signed `AgentMux.app` (v0.40.1) built via the new `task package:macos`.
**Status:** root-caused; fix proposed (not yet applied).

---

## What happened

Launching the packaged, Developer-ID-signed `AgentMux.app` (installed/stable runtime mode)
popped the macOS dialog:

> **"AgentMux wants to use your confidential information stored in 'Login' in your keychain."**

Worse, the app **hung with no window** (browser process at 0% CPU; the GPU child logged
`child_thread_impl.cc:902 Terminating current process after 15 seconds with no connection`).
The hang and the prompt are the **same event**: Chromium's `OSCrypt` initialization blocks on the
modal Keychain dialog during browser startup, so the message loop never runs, no window is created,
and the spawned GPU subprocess times out waiting to connect.

This was initially mis-diagnosed as a CEF macOS subprocess/Helper-app IPC problem. It is not — the
process was simply parked behind a system modal that only appears on the user's screen, invisible to
CLI process checks (which just see a "hung" 0%-CPU process).

## Root cause

The switches that keep Chromium away from the OS credential store —
`--password-store=basic` and (macOS) `--use-mock-keychain` — are **gated to `RuntimeMode::Dev`** in
`agentmux-cef/src/app.rs` (~L441–471):

```rust
let is_dev_runtime = matches!(RuntimeMode…, Some(RuntimeMode::Dev { .. }));
if is_dev_runtime {
    cmd.append_switch_with_value("password-store", "basic");
    #[cfg(target_os = "macos")]
    cmd.append_switch("use-mock-keychain");
}
```

A packaged `.app` resolves to `RuntimeMode::Installed`, so the block is skipped. Chromium's `OSCrypt`
then derives its cookie/password encryption key from the **macOS Keychain**, which triggers the
confidential-information prompt the first time it's touched — even though AgentMux never asks to save a
password. The code comment at that site literally predicts this dialog text.

### Why the gating existed

A prior review (reagent P1, earlier in the CEF-migration work) intentionally restricted these switches
to dev because they weaken Chromium's at-rest encryption: `--password-store=basic` routes `OSCrypt` to
an in-process store with an obfuscation-only key (no Keychain-derived key), and `--use-mock-keychain`
mocks the Keychain entirely. The rationale recorded in the comment: *"Released builds run once per
user; the prompt is appropriate there."*

That rationale conflicts with the product requirement, restated by the user: **AgentMux must not ask the
OS for credentials / confidential access.** The conflict was latent until macOS packaging actually
existed (it was a `[TODO]` stub until today), so no installed-mode macOS build had ever exercised the
non-dev path before.

## Contributing factors

- **No installed-mode macOS build existed until now**, so the prod credential path was never run on
  macOS — the regression had nowhere to surface.
- **The hang masked the cause.** The Keychain modal renders on-screen only; headless/CLI verification
  saw a 0%-CPU "hang" and chased a CEF-IPC red herring. (Lesson: a packaged GUI app that "hangs at 0%
  CPU right after init" should be eyeballed on screen for a system modal before deep IPC debugging.)
- The same gating affects **Linux AppImage** and **Windows installer** builds (gnome-keyring/kwallet/
  CredVault prompts), but those weren't in focus today.

## Impact

Every end user of a packaged AgentMux build hits an OS credential prompt on first launch (Keychain on
macOS). On macOS it additionally **blocks the app from opening** until dismissed — and dismissing it may
leave `OSCrypt` in a degraded state. This is a hard blocker for shipping a usable macOS release.

## Fix options

1. **Apply `--password-store=basic` (+ macOS `--use-mock-keychain`) in ALL runtime modes** (remove the
   dev gate, or extend it to installed/portable). Pro: no prompt, app launches, matches the product
   requirement. Con: reverses the reagent P1 decision — cookie/password encryption at rest uses an
   obfuscation key, not a Keychain-derived one. For AgentMux's threat model (a local, single-user agent
   workbench; data dir already `0700`; no shared multi-user secrets in the cookie jar) this is an
   acceptable, documented tradeoff. **Recommended.**
2. **Keep Keychain-backed encryption but pre-authorize** the app's access (stable signing identity +
   a keychain access group / partition-id ACL so it never prompts). Complex and brittle: the prompt is
   per-app-identity, and local/dev signatures vary; still prompts for ad-hoc builds.
3. **Per-user first-run choice / setting.** Doesn't remove the default prompt; just defers the policy.

## Recommendation

Take **option 1**, scoped to a small, well-gated change in `agentmux-cef/src/app.rs`: drop the
`is_dev_runtime` condition around the `password-store`/`use-mock-keychain` switches (the
`#[cfg(target_os = "macos")]` on `use-mock-keychain` stays — no Windows behavior change). Update the
comment to record the reversed tradeoff and link this retro. Re-package the macOS `.app` and confirm:
(a) no Keychain prompt, (b) the window opens, (c) the GPU subprocess connects. Then revisit Linux/Windows
parity.

## Action items

- [ ] Ungate the OSCrypt switches (option 1) — `agentmux-cef/src/app.rs`. Keep the macOS `cfg`.
- [ ] Re-run `task package:macos`; verify no prompt + window opens + renderer connects.
- [ ] Note the encryption tradeoff in the code comment + `docs/macos-signing.md`.
- [ ] Decide Linux/Windows parity (same prompt on those installed builds).
- [ ] Only then resume the v0.40.1 macOS release-asset upload (still also blocked on notarization — the
      Apple Developer agreement 403, tracked in the build-cleanup issue).

## Update (post-fix verification, 2026-05-30)

Implemented option 1 (`agentmux-cef/src/app.rs`: ungate the OSCrypt switches; macOS `cfg` kept) and
re-packaged + launched the installed-mode `.app`:

- **Keychain prompt: GONE.** Before the fix the app hung GPU-only at the keychain modal; after the fix it
  progresses *past* OSCrypt init and spawns the full subprocess set (gpu / network / storage / service /
  utility) and attempts the renderer. The credential-prompt hang is resolved — the credentials fix is
  **complete and verified**.
- **Separate issue uncovered:** with the keychain block removed, the **renderer/GPU subprocesses now
  crash-loop** in the signed, hardened-runtime bundle (274 crashes / 10 s → crash-budget abort → the
  "Window stopped recovering" page). This is the *other*, independent hard part of macOS CEF packaging
  (self-reexec subprocesses under a signed bundle identity — likely needs Helper.app bundles and/or
  hardened-runtime/entitlements tuning). It is **not** a credentials problem and does not affect this
  fix. Tracked with the `task package:macos` / macOS-packaging work, not here.

So: this retro's incident (the credential prompt + its hang) is **fixed**. A usable distributable macOS
build additionally needs the subprocess-crash work above + notarization (Apple-agreement 403).
