# Spec: Never request OS credential / keychain access (all runtime modes)

**Date:** 2026-05-30
**Status:** Spec → implementation
**Related:** `docs/retro/retro-macos-keychain-prompt-2026-05-30.md` (the incident this fixes)

---

## Problem

A packaged AgentMux build (installed/portable runtime mode) pops an OS credential prompt on first
launch — on macOS the **"AgentMux wants to use your confidential information stored in 'Login' in your
keychain"** dialog. On macOS this prompt is **modal and blocks browser startup**, so the app hangs with
no window until it's dismissed.

Root cause (`agentmux-cef/src/app.rs`): the Chromium switches that route `OSCrypt` away from the OS
credential store — `--password-store=basic` (all platforms) and `--use-mock-keychain` (macOS) — are
gated to `RuntimeMode::Dev`. Installed/portable builds skip them, so `OSCrypt` derives its cookie/
password encryption key from the platform-native backend (macOS Keychain, Linux gnome-keyring/kwallet,
Windows CredVault) and prompts the first time it's touched — even though AgentMux never saves a password.

The dev gate was a deliberate prior decision (the prompt was deemed "appropriate" for released builds,
and the switches weaken at-rest credential encryption). It was never exercised on macOS because
`task package:macos` was a TODO stub until 2026-05-30, so the conflict with the product requirement —
**AgentMux must not ask the OS for credential/keychain access in any build** — only just surfaced.

## Goal

No AgentMux build, in any runtime mode (Dev / Installed / Portable), on any platform, requests OS
credential or keychain access. The app launches without a system credential modal.

## Non-goals

- Keychain-grade at-rest encryption of the Chromium cookie jar (see Tradeoff).
- Changing how AgentMux's own auth/identity bundles are stored (that's app-level, already `0700`, and
  unrelated to Chromium's `OSCrypt`).

## Design

Remove the `is_dev_runtime` gate around the `OSCrypt` switches in `agentmux-cef/src/app.rs` so they
apply unconditionally:

- `--password-store=basic` — appended for **all** runtime modes and platforms. Routes `OSCrypt` to an
  in-process basic store instead of the platform-native backend → no gnome-keyring/kwallet/CredVault/
  Keychain prompt.
- `--use-mock-keychain` — appended for **all** runtime modes, **macOS only** (`#[cfg(target_os = "macos")]`
  stays). Belt-and-suspenders: even with `password-store=basic`, macOS `OSCrypt` still fetches its
  encryption key from the Keychain unless the keychain itself is mocked.

The `RuntimeMode` resolution that fed the gate is deleted (it's no longer needed). Comments are updated
to record the new, intentional behavior and link the retro.

### Why this is Windows-safe (regression guardrail)

- The change only **removes a condition** around two `cmd.append_switch*` calls — both are
  platform-agnostic `CefString` switch appends. `--password-store=basic` on Windows routes `OSCrypt`
  to the basic store (suppressing the CredVault prompt) — the desired behavior there too.
- `--use-mock-keychain` keeps its `#[cfg(target_os = "macos")]` — no change to what Windows/Linux compile.
- No type/state changes, no new platform-specific reads → no `#[cfg]`-shaped compile break (the failure
  mode of #1192). The diff is a net deletion of the gate.

## Tradeoff (explicit)

With `--password-store=basic`, Chromium's cookie jar is encrypted at rest with a hardcoded obfuscation
key rather than an OS-credential-derived key. For AgentMux's threat model this is acceptable:

- AgentMux is a **local, single-user agent workbench**; the data dir is already owner-only (`0700`).
- The cookie jar holds session cookies for the user's own logged-in services — not shared multi-user
  secrets — and an attacker with read access to a `0700` home dir has already lost regardless of OSCrypt.
- The product requirement (no OS credential prompts) is a hard constraint; this is the standard way CEF
  apps satisfy it.

This reverses the earlier "prompt is appropriate for released builds" decision; the reversal and its
rationale are recorded in the code comment + the retro.

## Implementation

- `agentmux-cef/src/app.rs`: delete the `is_dev_runtime` computation and the `if is_dev_runtime { … }`
  wrapper; append `--password-store=basic` (always) and `--use-mock-keychain` (always, macOS `cfg`).
  Rewrite the comment block.

## Verification

1. `cargo build -p agentmux-cef` (host compiles).
2. Re-run `task package:macos`; launch the signed `.app`:
   - **No** Keychain "confidential information" prompt.
   - The window **opens** (confirming the prompt-induced hang is resolved — same root cause).
   - A `renderer` subprocess spawns; `lsappinfo` shows the app `Foreground` with a Dock tile.
3. Spot-check Linux/Windows parity is unaffected at compile time (no `cfg` break).

## Rollout

- Land this as its own PR (the credential fix). It is a prerequisite for any usable packaged macOS
  build; the `task package:macos` tooling + notarization (Apple-agreement 403) are tracked separately.
