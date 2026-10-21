# Analysis: the macOS keychain prompt after an upgrade, and how to make it appear at most once

**Status:** analysis. Root cause confirmed on a real upgrade; the fix in §5 is not built yet.
**Date:** 2026-10-21
**Author:** Maop@starpower, at the operator's request
**Related:** [`SPEC_UPGRADE_FOR_USERS_AND_AGENTS_2026_10_15.md`](../specs/SPEC_UPGRADE_FOR_USERS_AND_AGENTS_2026_10_15.md) phase 3c (PR #5255), [`REPORT_UPGRADE_0_59_32_DOGFOOD_FINDINGS_2026_10_18.md`](../reports/REPORT_UPGRADE_0_59_32_DOGFOOD_FINDINGS_2026_10_18.md) (the keychain item it leaves open), [`retro-keychain-prompt-recurs-per-build-identity-2026-08-21.md`](../retro/retro-keychain-prompt-recurs-per-build-identity-2026-08-21.md), #4482 (srv's fixed signing identifier)

**Goal (operator):** the macOS keychain prompt appears at most once, right after the first MuxBus sign-in, and never because of an upgrade.

## 1. What happened

The end-to-end test of PR #5255 upgraded a notarized 0.59.35 build in place to the published 0.59.36. 0.59.36 opened the `stable` channel (it predates the channel record #5255 adds), and on its first start macOS showed "AgentMux Server wants to use your confidential information stored in "agentmux" in your keychain". srv's log shows its MuxBus token reads timing out every minute behind the unanswered prompt (`muxbus: keychain read failed: keychain read timed out after 15s`).

## 2. Root cause

**The prompt comes from srv's secret store reading a keychain item whose access list doesn't name srv's current signing identity.**

- securityd's log names the requester: `displaying keychain prompt for …/AgentMux.app/Contents/Helpers/AgentMux Server.app(<pid>)`, i.e. srv.
- The item is `service=agentmux, account=acct:muxbus:global`, the MuxBus sign-in of the `stable` channel (`backend/storage/muxbus.rs`: `stable` and an unset channel use the global namespace; every other channel has `acct:muxbus:channel:<channel>`).
- Its access list (`security dump-keychain -a`, attributes and ACL only) trusts five apps, each by a **versioned** code identifier: `agentmux-srv-0.59.1-darwin`, `-0.59.4-`, `-0.59.11-`, `-0.59.12-`, `-0.59.15-darwin`. Each was added by a click on "Always Allow" on an earlier prompt. Its partition is the team ID, which matches.
- srv has been signed with the fixed identifier `ai.agentmux.srv` since #4482 (2026-10-08), first shipped in **v0.59.16**. Release builds are signed by the same `scripts/package-macos.sh` (`build-macos.yml`). The designated requirements of the 0.59.34, 0.59.35 and 0.59.36 srv binaries are byte-identical.

So any item last trusted by a ≤0.59.15 srv asks once when a ≥0.59.16 srv first reads it. The operator's own 0.59.34 never asks, because its channel's entry (`acct:muxbus:channel:local-main-…`) was written by a ≥0.59.16 srv, and the writer is trusted.

## 3. What can prompt, and what can't

| Source | Prompts? | Why |
|---|---|---|
| srv's secret store (`identity/secret_store.rs`, `keyring` 2 on the legacy login keychain) | **Yes**, for an item whose ACL doesn't trust `ai.agentmux.srv` | Per-item ACL by designated requirement |
| The MuxBus sign-in at startup | The only secret read automatically at startup | `muxbus.rs`; the other secrets are read when they're used |
| Account API keys / OAuth accounts, browser-pane logins | Yes, on first use, for items written by ≤0.59.15 | Same store, one ACL per item |
| Model catalog's Claude token | Only from a user-started refresh, and only if it was saved from `CLAUDE_CODE_OAUTH_TOKEN` | `model_catalog.rs` `allow_keychain_fallback` |
| The embedded browser (CEF) | No | `--password-store=basic` and `--use-mock-keychain` (`crates/cef/src/app/mod.rs`) |
| Notifications | No | There's no macOS notification presenter (`launcher/src/notify/` has Linux and Windows only) |
| A `task dev` build | Yes, after each rebuild | Ad-hoc signed, so its identity changes every build; dev only |

**So, today:**
- **A user whose MuxBus sign-in (and accounts) were saved by v0.59.16 or later:** no prompt, now or after any later upgrade, as long as srv's designated requirement doesn't change.
- **A user with items from v0.59.15 or earlier:** one prompt per such item, the first time a newer srv reads it. That's once if they choose "Always Allow". If they choose "Allow" (this time only), it comes back on every start.
- **Each new local `task package` build:** gets its own channel, so it needs a new MuxBus sign-in. That's a sign-in, not a keychain prompt.

## 4. Gaps against the goal

1. **Legacy items ask, possibly several times.** One prompt per item from ≤0.59.15: the MuxBus entry, plus each saved account.
2. **"Allow" instead of "Always Allow" makes it permanent.** Nothing tells the user which to pick, and the prompt appears unasked-for at startup.
3. **The fixed identity can regress unnoticed.** It already did once: the August fix was lost and only restored by #4482. A release with a different srv identity would make every user's every item ask again.
4. **A write to a legacy item** (a token refresh, a re-sign-in) can also need consent, and writes have no timeout (`secret_store.rs`), so an unanswered prompt blocks the writer.

## 5. Plan: at most one prompt, only right after signing in

1. **Never show the consent UI from a background or startup read.** Turn keychain user interaction off for srv's reads (`SecKeychainSetUserInteractionAllowed(false)` for the legacy keychain, or `kSecUseAuthenticationUIFail` on the `SecItem` path). A read that would need consent then fails at once with `errSecInteractionNotAllowed`, instead of opening a dialog or waiting 15 s.
2. **A MuxBus entry that would need consent counts as signed out.** The app shows "Sign in to MuxBus again" once. The new sign-in writes a **fresh** entry, created by `ai.agentmux.srv`, which is trusted from then on. The old entry is deleted if the keychain allows it, otherwise the new one goes under a new account name. Result: zero prompts after the sign-in, and none after any later upgrade. That is within the goal, since the sign-in is the user's own action.
3. **For account API keys and OAuth accounts** (where signing in again means re-entering keys), offer one explicit "Allow AgentMux to use this account's key" action. It does the one read that shows the prompt, says to choose "Always Allow", and rewrites the item so its creator is `ai.agentmux.srv`.
4. **Guard the identity in CI.** `package-macos.sh` (and the release workflow) should fail when srv's designated requirement isn't `identifier "ai.agentmux.srv"` plus the team, so gap 3 can't ship again.
5. **Writes:** do the first write to a legacy item with interaction off too. If it fails, fall back to delete-and-create (or the new account name), never an unbounded wait on a hidden dialog.

**Needs a spike before building:**
- whether the `keyring` 2 / `security-framework` path exposes the interaction switch, or needs a small FFI;
- whether deleting a legacy item another identity created needs consent (its ACL lists `change_acl` with no apps, and no explicit delete entry).

## 6. The Vault

The Vault ([`SPEC_VAULT_PANE_AGENTS_USE_SECRETS_UNSEEN_2026_10_19.md`](../specs/SPEC_VAULT_PANE_AGENTS_USE_SECRETS_UNSEEN_2026_10_19.md) §5.5, first shipped in v0.59.36) keeps **one** random 256-bit key in the secret store, and seals every entry with it in the identity store (`backend/vault/seal.rs`). It does so on purpose, because macOS asks about each keychain item separately.

- **Its key is always written by `ai.agentmux.srv`** (the Vault postdates v0.59.16). By §2, a signed build should therefore never be asked about it: not on the first save, and not after an upgrade. Not yet confirmed on macOS by running the Vault.
- **The note the Vault pane shows (#5205)** says that on macOS the keychain asks "the first time AgentMux uses the Vault key, and again after AgentMux updates". For signed builds, neither should happen. It should say a prompt can come once, and only for an unsigned or `task dev` build.
- **`task dev` builds** share the same key (the identity store and the keychain are machine-wide), and their ad-hoc identity changes every rebuild. So each dev rebuild asks once, and Deny leaves the Vault unopenable in that build.
- **It is the model for §5.** Seal the other secrets (MuxBus tokens, account API keys and OAuth, browser-pane logins) with the Vault key too, and AgentMux has one keychain item, so at most one prompt ever. Moving legacy items then becomes a one-off step:
  - MuxBus: sign in again (§5.2).
  - Accounts: read each once with consent (§5.3), seal it, and delete the old item.
- **Reveal and Copy** are meant to ask for Touch ID on macOS. That's not built yet, so they're disabled there. When built, it's a per-use confirmation by design, separate from keychain trust.

## 7. Other open questions from the macOS upgrade, resolved

- **Do macOS privacy permissions survive an upgrade?** Mostly not relevant to normal use.
  - The browser doesn't use the keychain, and there are no macOS notifications.
  - Grants attributed to the main app don't carry across versions, because every version has its own bundle id (`ai.agentmux.<channel>.<version>`). Seen on one Mac: Screen Recording granted to 0.59.28 was not granted for 0.59.34.
  - Grants attributed to srv's fixed identity do carry (Accessibility was granted for 0.59.34's process tree).
  - Not verified: Local Network (LAN discovery) and Files & Folders, which depend on which process macOS holds responsible.
- **A fixed bundle id for stable releases?** Not needed for the keychain: srv's identity, not the bundle id, decides keychain trust. Keep per-version bundle ids (they keep two stable versions launchable side by side) unless Local Network or Files & Folders turn out to reset on every upgrade. Test that next with two notarized builds.
- **"Pre-migration snapshots: the last 5 are kept"** (agentmux-docs `multi-instance.md`): correct; `MAX_SNAPSHOTS_PER_CHANNEL = 5` (`backend/storage/snapshot.rs`).

Agent: Maop@starpower
