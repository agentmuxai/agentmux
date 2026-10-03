# SPEC: share authentication across channels — no re-login on every build

**Date:** 2026-10-03
**Status:** proposed — not built. §7 lists the decisions that need the owner before Phase 1.
**Author:** AgentY, at the owner's request
**Amends:** `SPEC_ISOLATED_AUTH_DEFAULT_BY_CHANNEL_2026_08_06.md` (reverses its default), `SPEC_ISOLATED_AUTH_DEV_TESTING_2026_07_27.md` (the opt-in mechanism stays), and the per-channel half of `SPEC_MUXBUS_KEYCHAIN_PER_CHANNEL_2026_10_02.md` (#4190).
**Related:** `docs/analysis/ANALYSIS_PER_CHANNEL_AUTH_BYPASSES_2026_08_31.md` (invariant INV-PC), `SPEC_IDENTITY_STORE_SPLIT_2026_08_17.md`, `SPEC_AGENT_SINGLE_LIVE_INSTANCE_2026_09_24.md`, `SPEC_MUXBUS_CROSS_CHANNEL_DUPLICATE_DELIVERY_2026_07_04.md`, `SPEC_LOCAL_CHANNEL_PRUNER_2026_06_25.md`, `SPEC_MY_AGENTS_TILES_AUTH_AND_HISTORY_2026_10_03.md` (this spec is its prerequisite).

## 1. The request (owner, 2026-10-03)

> Make authentication non-channel-isolated, so channels can share the auth. We isolated auth to channels to get rigorous dogfooding of login in, but it appears stable at this point. So we don't need to log in on every upgrade. And that way My Agents can retain more information in the agent pane. Do this first.

## 2. How it works today

### 2.1 What a channel is, and who gets isolated

A channel is one build's own data folder, `~/.agentmux/channels/<channel>/`. The rule (`isolated_auth_reason()`, `crates/common/src/data_paths.rs:787-801`):

| Channel | Auth |
|---|---|
| `AGENTMUX_ISOLATED_AUTH=1` | isolated (explicit opt-in) |
| `AGENTMUX_ISOLATED_AUTH=<anything else>` | shared (explicit opt-out) |
| no override, channel is `stable` or unset | **shared** |
| no override, any other channel | **isolated** (the 2026-08-06 default) |

- **Releases bake `stable`** (`RELEASE_CHANNEL=stable` in the Windows, macOS and Linux build workflows). An installed release already shares auth.
- **Every `task package` build makes a new channel**: `local-<branch>-<branch-hash>-<8 hex of the build label>` (`scripts/package.sh:102-103`; the same in `package-macos.sh` and `package-linux.sh`). The portable on the owner's Desktop (`agentmux-0.59.4+g0528545b5.…-x64-portable`, a label in the name) is one of these, so **each new build starts logged out**.
- **`task dev` uses `dev-<branch>-<clone>`**, stable across rebuilds of one branch and clone, so the re-login there is per new branch or clone.
- `~/.agentmux/channels/` holds **46 per-channel identity stores** today, nearly all `local-main-b28b7a-*`. Nothing prunes them: the pruner spec is still Draft and no pruner code exists.

### 2.2 The flag moves far more than logins

`isolated_auth_enabled()` relocates one physical store and several derived things. Callers (from a full read of the source):

| What | Where it lives | Per channel when isolated |
|---|---|---|
| Accounts (`db_accounts`) | `id_store` = `<instance>/identity-store.db`, else `shared/store.db` (`registry/paths.rs:64`) | yes |
| Provider login folders | `identities_dir()` = `channels/<ch>/identities`, else `shared/identities` (`data_paths.rs:399`) | yes |
| Memory bundles, drones, native memory | same `id_store` | yes |
| **Global Memory** | scope `channel:<id>` vs `shared` (`backend/global_memory_record.rs:51`) | yes |
| MuxBus sign-in row and **per-agent cloud credentials** (`db_agent_credentials`) | same `id_store` | yes |
| MuxBus **tokens** (OS keychain) | `muxbus:channel:<ch>` vs `muxbus:global` (`storage/muxbus.rs:24-38`, #4190) | yes |
| Account → agent **links** (`db_agent_identity_links`) | `identity_store` = `shared/identity-store.db` | **no, always global** |
| Agent definitions, registry, **transcripts**, leases | `shared/agents/…` | **no, always global** |
| Settings | a separate flag, `AGENTMUX_ISOLATED_SETTINGS` | separate |

### 2.3 Two consequences you can see

- **"(missing account)" tiles.** The account links are global but the accounts are not, so an isolated channel has links to accounts it cannot find (`SPEC_MY_AGENTS_TILES_AUTH_AND_HISTORY_2026_10_03.md` §2.2).
- **Stranded data.** The Global Memory entry "GitHub and AWS access" that went missing is not lost: a read-only search of the stores found it, and only it, in `channels/local-agent1-release-patch-bump-abe85c-306df097/identity-store.db`, a build from another branch that none of the owner's current windows read. Every per-build channel hides the Global Memory, bundles and drones made in every other.

## 3. Why it was isolated, and what has changed

2026-08-06: a developer could not test the login and relogin paths because every `task dev` inherited the owner's 12 real, logged-in Claude accounts, and deleting or disconnecting one to force a relogin would have signed out real agents. Isolation made every new channel exercise a real login. The cost was named then: "someone's `task dev` now asks them to log in again" (§4 of that spec).

The owner's call now is that login is stable and the daily cost is no longer worth it. The explicit switch stays, so the testing use survives as an opt-in (§4.1).

Hazards sharing brings, all verified in the source (§6 turns each into a requirement):

| Hazard | Today |
|---|---|
| **Newer schema stamps the shared store.** `check_schema_compat` refuses a database whose `user_version` is newer than the build (`storage/migrations.rs:2352`), and `id_store` then **silently falls back to the per-channel store**, so an older build looks logged out. Releases are the oldest builds in daily use, local `main` builds are the newest | new |
| **A development build can sign out or delete the real login.** Account delete removes the shared credentials; MuxBus sign-out deletes the shared tokens (`muxbus_clear`) | new for dev channels |
| **The channel pruner (unbuilt) would delete credentials** an adopted account row points into | design constraint |
| **Two channels signed in to one cloud account both receive a jekt** (`SPEC_MUXBUS_CROSS_CHANNEL_DUPLICATE_DELIVERY_2026_07_04.md`, Draft) | the single-live-instance relay lease already fences a second instance's pulls (`wan_lease`, shipped); shared sign-in must be tested against it |
| Token rotation across channels | **not a hazard here**: AgentMux never refreshes provider tokens (`oauth_probe.rs` only reads `expiresAt`), and Cognito does not rotate the MuxBus refresh token (`pkce.rs:359`). The documented rotation failure was *copying* a login; sharing one folder is not copying |
| Two servers on the shared stores | SQLite WAL with `busy_timeout=5000`; the global migration lock already sits beside the shared store. This is what `stable` and two live versions already do |
| The orphan-account-folder sweeper (`identity/cleanup.rs:225`) | safe when rows and folders are both shared; it is only dangerous in the mixed state (isolated rows, shared folders), which this spec never creates |

## 4. Requirements

R1. **Auth is shared by default on every channel.** A new build, a rebuilt branch, `task dev` and a release all read the same accounts, provider login folders, bundles, drones, native memory, Global Memory, MuxBus sign-in and per-agent cloud credentials, i.e. everything the one flag controls today. No login on upgrade.

R2. **Isolation stays available, explicitly.** `AGENTMUX_ISOLATED_AUTH=1` still isolates, for testing login and relogin. Tests and CI are unaffected (they already set their own home).

R3. **A channel that was isolated keeps what it had.** The first boot in shared mode **adopts** the previous isolated state into the shared store, without deleting or modifying the source and without losing a working login (§5.2).

R4. **Sharing never breaks an older build.** A change that makes a shared store unreadable by any build still in use is refused before it ships (§5.4).

R5. **A development build cannot silently destroy the shared login.** Deleting an account, disconnecting a login or signing out of the cloud from a non-`stable` channel asks first, and says the login is shared (§5.4).

R6. **The My Agents list sees consistent accounts.** With accounts and links in one scope the "(missing account)" case disappears except for a link to a genuinely deleted account (handled by the tiles spec).

## 5. Design

### 5.1 The default

Remove the channel rule. `isolated_auth_reason()` becomes: `AGENTMUX_ISOLATED_AUTH=1` → isolated; any other value or unset → shared. `IsolatedAuthReason::ChannelDefaultIsolated` is deleted, and the boot log line (`bootstrap/stores.rs:405`) reports one of `shared`, `shared (explicit opt-out)`, `isolated (explicit opt-in)`. The two sibling flags are not touched: **settings stay per channel** unless the owner decides otherwise (§7.5), and the MuxBus eager-reconnect flag (`AGENTMUX_ISOLATED_MUXBUS`, which skips connecting at boot on `local-*` builds so a new code signature does not prompt the keychain) stays as it is.

Because #4190 made the keychain namespace follow the same flag ("exactly when the row is per channel", `storage/muxbus.rs`), flipping the flag moves the MuxBus tokens back to `muxbus:global` with no separate code change. The adoption below covers the tokens that only exist under a channel namespace.

`task dev` needs no change. A documented `AGENTMUX_ISOLATED_AUTH=1 task dev` replaces the old default for someone testing login.

### 5.2 Adopting the previous isolated state

The danger is a one-time double login that is *worse* than today's: the owner's current window holds real logins in `channels/local-main-b28b7a-051fbf53/`, and the first shared build would see an empty shared store. So adoption is part of Phase 1, not a follow-up.

**What a shared-mode boot adopts, and from where.**

- **Sources:** (a) this channel's own isolated store, when one exists, which is the case for a `dev-<branch>-<clone>` channel that persists across rebuilds; (b) for a per-build `local-…` channel, the **newest other channel with the same branch prefix** (everything before the last 8-hex build suffix, e.g. `local-main-b28b7a-`), the user's predecessor build. Nothing else is adopted automatically: with 46 stores on disk, merging all of them would flood the Armory with test accounts (QTest, ScrollPinTest-fresh-D4, …) and duplicates.
- **What:** accounts (`db_accounts`), memory bundles and versions, drones, native memory, Global Memory entries, the MuxBus sign-in row, and per-agent cloud credentials (`db_agent_credentials`).
- **How:**
  - read-only on the source, never moved, never deleted;
  - every adopted row keeps its UUID, so a second pass is a no-op, and the union of two stores is safe;
  - a `db_adoptions` table in the shared store records `(source path, source modified time, adopted at)` so an unchanged source is not re-read;
  - a name collision (Global Memory, bundles) keeps the newest by its updated-at and writes the other into the entry's version history (Global Memory already keeps history), never overwriting silently;
  - an account row keeps its `OAuthConfigDir` **pointing at the source channel's folder** (`channels/<old>/identities/<uuid>/claude`). The spawn path uses the stored folder and does not re-derive it (`inject.rs:765`), and the shared store already holds rows pointing into per-channel folders (`ANALYSIS_PER_CHANNEL_AUTH_BYPASSES_2026_08_31.md`, bypass 1). The login works the instant the build starts, with no copy, and **a copied login is exactly what caused the documented "Pozl 401"**. New logins go to `shared/identities`.
- **MuxBus tokens:** when `muxbus:global` has no tokens and the channel's own `muxbus:channel:<ch>` namespace does, copy them to `muxbus:global` (the reverse of the adoption #4190 added), read-only on the source, and never overwrite tokens already in `muxbus:global`.

**Import on demand.** An Armory action, "Import from another channel…", lists the other channels' stores with their account counts and last-used times, and adopts the chosen ones with the same rules. That is also how the stranded "GitHub and AWS access" entry comes back: its channel is another branch's, so it is not adopted automatically.

**Adopted folders and the pruner.** Referencing in place means a source channel's `identities/` folder is now live data. The pruner (`SPEC_LOCAL_CHANNEL_PRUNER_2026_06_25.md`, unbuilt) deletes dead `local-*` channels' `data/`, `cef-cache/` and `logs/` and never mentions `identities/`. This spec adds the rule to it: **a channel folder referenced by any account row in the shared store is never pruned**, and a later maintenance step may move such a folder into `shared/identities` and rewrite the row, only when no server holds that channel's data-dir lock. That move is not part of Phase 1.

### 5.3 MuxBus and the cloud

- **One sign-in, one set of tokens, one row**, all in the shared scope. This restores the consistency #4190 was missing from the other side: its failure was a per-channel row over host-wide tokens. With both shared there is no mismatch. Sign-out signs out every channel, which is now correct because they share the sign-in.
- **Duplicate jekt delivery.** Two channels signed in to one account will both subscribe their live agents. The relay's lease already refuses a second instance's pull for an agent it does not hold (`wan_lease`, `SPEC_AGENT_SINGLE_LIVE_INSTANCE_2026_09_24.md` Phase 5), and the shared-sign-in test in §8 asserts it. A dev build's agents that are not also live on the main build are unaffected.
- Per-agent cloud credentials live in the same shared store, keyed by agent id, so two channels use one credential per agent instead of provisioning a second one. That is a saving: each new agent id consumes the account's `agent_provisions` allowance, and a per-channel store re-provisions the same agent in every channel.

### 5.4 Mitigations for the new hazards

- **Schema compatibility (R4).** Shared-store migrations are **additive only** (new tables, new nullable columns, new indexes); a destructive or type-changing change needs an expand-then-contract pair spread across two releases. A CI test opens a database stamped with the *next* migration using the previous build's reader and expects it to open (the reader ignores a `user_version` newer than it understands as long as the tables it needs are present), replacing the refuse-and-fall-back-silently behaviour for shared stores. Until that lands, a build that cannot open the shared store **says so in the status bar and the Armory** instead of falling back to an empty per-channel store that looks like a logout.
- **Destructive auth operations from a non-`stable` channel (R5).** Deleting an account, disconnecting a login and MuxBus sign-out show "This login is shared by every AgentMux on this computer." with the channel name, and require confirmation. A `stable` channel behaves as today.
- **The sweeper and the orphan logic** need no change in shared mode (§3). The two INV-PC tests that assert a spawn refuses an account that exists only in another channel's mirror stay valid, because in shared mode the account is in the channel's own (shared) store; they gain a variant under explicit isolation.

### 5.5 Docs and tests

Update the isolation specs and the stale "isolated by default for every non-stable channel" statements in the per-build channel specs (invariant I6 in the Linux AppImage and macOS DMG specs), the comments in `scripts/package.sh`, and `SPEC_MUXBUS_KEYCHAIN_PER_CHANNEL_2026_10_02.md` (which this spec supersedes for the default case). A changeset records the behaviour change and the one-line opt-in.

## 6. Phases

| Phase | What | Risk |
|---|---|---|
| 0 | Dry-run adoption against a **copy** of the owner's real `~/.agentmux` (the 46 stores) and report what would be adopted, with no writes | none |
| 1 | The default flip, adoption (§5.2) and the keychain token adoption, the boot-time "cannot open the shared store" notice, the Armory "Import from another channel…" action. One PR; the flip and the adoption must ship together | medium: touches every login |
| 2 | Additive-only migration rule and its CI test (§5.4); confirmation on destructive auth operations from non-`stable` channels | low |
| 3 | The pruner guard (a reference check) when the pruner is built; the optional folder consolidation | low |
| 4 | Docs sweep and changeset | none |

Phase 0 first: adoption is the only step that can lose a login, and it should be seen on real data before it runs on the owner's machine.

## 7. Decisions for the owner

1. **Every channel, or only some?** I assumed *every* channel, including `task dev`, with isolation as an explicit opt-in. The alternative is to share for `local-*` and release builds and keep `dev-*` isolated. The cost of "every channel" is that a `task dev` branch can now touch the real login, which §5.4's confirmation is meant to cover.
2. **Adoption source.** The newest same-branch predecessor plus on-demand import (§5.2), or adopt every channel's accounts automatically? I recommend the former because of the 46 stores and the test accounts in them.
3. **Global Memory, bundles and drones become shared too.** They follow the same flag, so after this a "GitHub and AWS access" entry made in any build is visible in all of them. Is that what you want, or should those stay per channel? (Splitting them is the identity-store split that never finished, `SPEC_IDENTITY_STORE_SPLIT_2026_08_17.md`, and is larger work.)
4. **Dev-build confirmation (R5).** A confirmation on destructive auth operations in non-`stable` channels, as proposed, or nothing?
5. **Settings** stay per channel under their own flag in this spec. Share them too?
6. **Tell Manoz.** This reverses the default half of #4190 (his per-channel MuxBus keychain work, one day old). I would send him a note before the PR; say if you would rather I did not.

## 8. Tests

1. `isolated_auth_reason()` and its callers: shared with no override on `stable`, `local-…`, `dev-…` and an unset channel; isolated only with `=1`; `=0` and malformed values shared. Replaces the channel-default tests.
2. The keychain namespace follows the flag: `muxbus:global` on every channel by default; `muxbus:channel:<ch>` only under `=1`.
3. Adoption: two channels' stores merge by UUID; a second boot is a no-op (`db_adoptions`); the source files are byte-identical afterwards; a name collision keeps the newest and records the other in history; an adopted account's `OAuthConfigDir` still points at the source folder and a spawn using it succeeds; a source that fails to open is skipped with a warning and the boot continues.
4. Tokens: channel-namespace tokens are copied to `muxbus:global` only when it is empty, never over existing ones.
5. Schema skew: a build opens a shared store stamped with a newer additive migration and reads its own tables; a store it genuinely cannot read produces the visible notice, not a silent per-channel fallback.
6. Shared sign-in with the lease: two server instances share one sign-in, and only the instance holding an agent's lease receives its jekts.
7. A destructive account operation from a non-`stable` channel needs the confirmation and names the channel.
8. The two INV-PC spawn tests, run under explicit isolation, still refuse another channel's account; in shared mode the same account spawns.
9. Pruner guard (when the pruner exists): a channel folder referenced by an account row is skipped.
10. Phase 0's dry-run is a test fixture: a copy of a representative multi-channel `~/.agentmux` shape, with stores in the real schema.

## 9. Not in this spec

Per-channel **settings** (a separate flag and spec). Splitting Global Memory, bundles and drones out of the account store (the unfinished identity-store split). Migrating existing provider login folders into `shared/identities`. The My Agents tile changes themselves, which follow this spec.
