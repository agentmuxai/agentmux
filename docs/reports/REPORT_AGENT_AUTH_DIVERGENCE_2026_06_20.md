# REPORT: Why one agent authenticates and another doesn't — and why "Login Again" is a no-op

**Date:** 2026-06-20
**Author:** AgentA
**Status:** Diagnosis complete; fix proposed (not yet implemented)
**Related:** `SPEC_HOST_CLI_LOGIN_CAPTURE_2026_06_20.md` (failure-mode A), `SPEC_REAUTH_FROM_AUTH_ERROR_2026_06_20` (#1604), cross-channel persistence work (#1383–#1393)

---

## 0. TL;DR

Two Claude agents — **Nark** (just created, works) and **Poal** (created ~Jun 18, 401s) — are bound to the **same** "Default" identity, which in both their instance DBs resolves to a **valid** credential (`dir: C:\Users\area54\.claude`, `status: valid`). They diverge only because **an agent's `CLAUDE_CONFIG_DIR` is frozen into its launch env at first launch and never re-resolved**:

- **Nark** froze pointing at the live global login (`~/.claude`).
- **Poal** froze pointing at the isolated `~/.agentmux/shared/providers/claude/` dir — which is **expired/dead**.

"Login Again" does nothing because it (a) scrapes Claude v2.1.x's self-driving login TUI for an OAuth URL it never prints, and (b) even on success would write to Poal's stale frozen dir, not the valid global. The fix is to **reuse the create-time "existing login" resolution** (Default bundle → global `~/.claude`) in the recovery path instead of the broken scrape — the DRY ask.

---

## 1. Scenario

In the v0.46.6 window the user sees two Claude agents:

- **Nark** — just created — authenticates and responds normally.
- **Poal** — around since ~Jun 18 — fails to authenticate; clicking **Login Again** does nothing.

Both are the *same* provider (`claude`) and, as shown below, the *same* identity. So "they share one login" is **false** — and "one login is just expired" is also too simple.

---

## 2. Q1 — How is one agent authed while another isn't?

### 2.1 Both are bound to the same "Default" identity → a *valid* account

From each agent's instance DB:

| Fact | Nark (`.6` channel DB) | Poal (`dev/agenta-template-runtime-choice` DB) |
|---|---|---|
| `db_agents.identity_id` | `default` | `default` |
| binding `default→claude` account | `1615e7ae-…` | `1a1778c9-…` |
| account `secret_ref` | `{backend: o_auth_config_dir, dir: "C:\Users\area54\.claude"}` | `{backend: o_auth_config_dir, dir: "C:\Users\area54\.claude"}` |
| account `status` | `valid` | `valid` |

So per the identity system, **both resolve to your valid global `~/.claude` login.** On paper they are identical.

### 2.2 The real divergence: the credential dir is *frozen at launch*

An agent's `CLAUDE_CONFIG_DIR` is captured into its block meta (`cmd:env`) at first launch and reused verbatim on every turn and on "Login Again". It is **not** re-resolved from the identity binding afterward.

From Poal's block meta (`db_block` in its instance DB):

```
"CLAUDE_CONFIG_DIR":"C:\\Users\\area54\\.agentmux\\shared\\providers\\claude"
```

- **Poal** (`working_directory = ~/.agentmux/agents/poal-0618g`, `started_at = 1781824185672` ≈ Jun 18) froze pointing at the **isolated shared dir** `~/.agentmux/shared/providers/claude/`.
- **Nark** (created just now) froze pointing at the **valid global** `~/.claude` (the Default-bundle account dir).

The **resolution logic changed** between Jun 18 and now: earlier launches resolved Claude's config dir to the isolated `shared/providers/claude` (via the frontend's `ensureAuthDir` → host `ensure_auth_dir`), while later launches resolve via the Default-bundle migration to the global `~/.claude`. Poal is frozen on the old answer; Nark on the new one.

### 2.3 Why the frozen dir is dead

On-disk state of `~/.agentmux/shared/providers/claude/.credentials.json` (Poal's frozen dir):

- **As-found (before any test):** `expiresAt = 1781831066028` (≈ Jun 19, **past**), `refreshToken` **empty**, `accessToken` 29 chars. → expired, **cannot refresh** → 401 / needs-reauth.
- The global `~/.claude/.credentials.json` (Nark's dir): `expiresAt = 1781978253240` (**future**), has `refreshToken` → **valid**.

> **Test-artifact disclosure:** during verification a forced-401 was written into that same shared dir (`accessToken` = `sk-ant-oat01-FORCED-401-VERIFICATION-DO-NOT-USE`, future expiry, empty refresh). That dir is Poal's frozen dir, so the corruption *lands on* Poal — but the dir was **already dead before the test**, so it is not the cause of Poal's failure, only a coincident overwrite. Restorable from `~/.agentmux/shared/providers/claude/.credentials.verify-backup.json`.

### 2.4 Answer

Same user, same "Default" identity, but **each agent froze a different `CLAUDE_CONFIG_DIR` at launch**. Nark's points at the live global login; Poal's points at the isolated shared dir, which is expired with no refresh token. This is a **cross-instance / temporal binding-consistency gap**: the identity→dir binding moved forward (Default bundle → `~/.claude`) but already-launched agents kept their stale frozen env.

---

## 3. Q2 — Why does "Login Again" do nothing?

Path: failure-row **Login Again** → `relogin()` (`useAgentControllerStatus.ts`) → `forceProviderLogin()` (`frontend/app/view/agent/flows/force-login.ts`) → `getApi().runCliLogin(cliPath, authLoginCommand, authEnv, requiresTty)` → host `run_cli_login_pty` (`agentmux-cef/src/commands/platform.rs`).

Two compounding reasons it no-ops for Poal:

1. **No URL to scrape.** `run_cli_login_pty` spawns `claude auth login` in a PTY and scans stdout for an OAuth URL. Claude Code **v2.1.183** runs a self-driving login TUI: it opens its own browser and **clipboard-copies the URL on `c`** — it never prints a scrapeable `https://…` line. So `extract_url` captures nothing → no browser opens, no auth box appears → "nothing happens." (Spec failure-mode A.)
2. **Wrong write target even on success.** `forceProviderLogin` reuses Poal's frozen `authEnv` (`CLAUDE_CONFIG_DIR = …/shared/providers/claude`). Even if a login completed, the fresh token would land in the **dead shared dir**, not the valid global — so the fix wouldn't stick.

---

## 4. How agent auth actually resolves (reference)

| Stage | Where | What happens |
|---|---|---|
| **Create-time seed** | `agentmux-srv/src/identity/migration.rs` `run_default_bundle_migration()` (~l.92), `ensure_default_bundle()` (~l.409) | On srv startup, probes global `~/.claude`; if usable, creates the **"Default"** bundle + a `claude` account whose `secret_ref` is `OAuthConfigDir { dir: ~/.claude }`, and binds it. New agents get `identity_id="default"`. |
| **Bundle dir compute** | `agentmux-srv/src/server/identity_handlers.rs` `compute_and_ensure_bundle_dir()` (~l.950) | For an explicit bundle, overrides `CLAUDE_CONFIG_DIR` to the per-bundle dir. |
| **Default auth dir** | `agentmux-cef/src/commands/platform.rs` `ensure_auth_dir()` (l.79) | Returns the shared default `~/.agentmux/shared/providers/<provider>/`. This is the **legacy** target older agents froze. |
| **Spawn-time inject** | `agentmux-srv/src/identity/resolver.rs` `inject_identity_env()` (~l.369), `probe_oauth_status()` (~l.85) | Resolves the binding → sets `CLAUDE_CONFIG_DIR` to the account's dir; probes expiry. |
| **Frozen launch env** | frontend `launch-flow.ts` SetMeta (~l.159), `buildAuthEnv()` in `useAgentControllerStatus.ts` (~l.95) | The resolved `CLAUDE_CONFIG_DIR` is written to block meta `cmd:env` **once** and reused thereafter (incl. by `relogin`). **This is where the staleness is frozen in.** |
| **Re-auth** | `force-login.ts` `forceProviderLogin()` (~l.43) | Reuses the frozen `cmd:env` + the broken OAuth-URL scrape. No re-resolution, no reuse of the create-time seed. |
| **Seed-from-global (new, #1613)** | `agentmux-cef/src/commands/providers.rs` `seed_provider_auth_from_global()`; frontend `flows/seed-global-login.ts`; 🌐 CTA | Copies a valid global `~/.claude` cred into the **hardcoded** shared dir. |

On-disk layout:

```
~/.claude/.credentials.json                                   ← global login (VALID)
~/.agentmux/shared/providers/claude/.credentials.json         ← shared default (Poal's frozen dir; DEAD)
~/.agentmux/shared/identities/<bundle_id>/claude/.credentials.json  ← per-bundle (NONE present here)
```

---

## 5. Bug exposed in the shipped seed-from-global (#1613)

`seed_provider_auth_from_global()` hardcodes its destination to `~/.agentmux/shared/providers/claude/`. That **happens to match Poal** (so clicking 🌐 would fix Poal today), but it would **silently miss** any agent whose resolved dir is a bundle or the global path (e.g. Nark → `~/.claude`). The seed must target the **agent's resolved `CLAUDE_CONFIG_DIR`** (passed from the caller), not a constant.

---

## 6. Proposed fix — DRY the "existing login" into recovery

**Principle:** the "login an agent gets when first created" *is* the Default bundle → global `~/.claude`. Recovery (Login Again / the auth-failure CTA) should **reuse that exact resolution** instead of the OAuth scrape.

Concretely, one shared "use the existing global login" path serving **create**, **Login Again**, and **🌐**:

1. **Re-resolve, don't reuse-frozen.** On auth failure, re-run the identity resolution for the agent (`inject_identity_env` / the Default-bundle account dir) and **refresh its `cmd:env` `CLAUDE_CONFIG_DIR`**, so a stale agent picks up the same live dir a fresh agent gets. Fixes the temporal-staleness root cause (§2.2).
2. **Fix the seed target.** Make `seed_provider_auth_from_global` accept the agent's **resolved** dir (frontend passes `cmd:env.CLAUDE_CONFIG_DIR`) instead of the hardcoded shared dir (§5).
3. **Wire recovery to seed first, scrape last.** For Claude (un-scrapeable TUI), the auth-failure recovery should try the seed-from-global into the resolved dir **before** falling back to the OAuth flow — making "Login Again" actually succeed without a second button.
4. **(Optional, deeper)** Globalize the identity→dir binding so the same "Default" identity resolves consistently across instances/channels (ties into #1383–#1393). Without this, re-resolution still depends on the running instance's DB.

Trade-off to decide: **re-point env** (1) vs **seed file** (2) vs **both**. Recommended: both — re-point so the agent uses the live dir, and seed so that dir is guaranteed populated with a valid cred.

---

## 7. Immediate remediation options (for the live Poal)

- **Click 🌐 "Use existing login" on Poal's failure row.** Poal's frozen dir is the shared dir, which is exactly where the current seed writes — so this should copy the valid global login in and recover Poal on the next message. (If the 🌐 button doesn't render — Poal's process lives in a dev instance — seed from CLI instead.)
- **CLI seed:** copy `~/.claude/.credentials.json` → `~/.agentmux/shared/providers/claude/.credentials.json` (overwrites the dead/forced-401 cred with the valid global).
- **Restore the test artifact:** `~/.agentmux/shared/providers/claude/.credentials.verify-backup.json` holds the as-found (dead) cred if a literal restore is wanted — but seeding the valid global is the actual fix.

---

## 8. Evidence appendix (commands used)

```
# identity binding (per instance DB)
sqlite3 <objects.db> "SELECT identity_id FROM db_agents WHERE name IN ('Nark','Poal');"
sqlite3 <objects.db> "SELECT * FROM db_identity_bindings WHERE provider='claude';"
sqlite3 <objects.db> "SELECT id,provider,status,secret_ref FROM db_identity_accounts WHERE provider='claude';"

# Poal's frozen launch env
sqlite3 <poal objects.db> "SELECT * FROM db_block;" | grep -o 'CLAUDE_CONFIG_DIR":"[^"]*'
#  → C:\Users\area54\.agentmux\shared\providers\claude

# credential validity (no token values printed)
#  ~/.claude/.credentials.json                          expiresAt future=true,  hasRefresh=true   (VALID)
#  ~/.agentmux/shared/providers/claude/.credentials.json expiresAt future=false, refreshToken=""  (DEAD)
```

DB locations:
- Nark: `~/.agentmux/channels/local-main-b28b7a-6e60a938/versions/0.46.6/data/db/objects.db`
- Poal: `~/.agentmux/dev/agenta-template-runtime-choice/69d7a34a544eaf3e/data/db/objects.db`

---

## 9. History / provenance — how auth drifted from "isolated" to "shared-global"

> Context: AgentMux was originally designed so each provider kept its **own**
> AgentMux-owned auth, **decoupled from the user's personal `~/.claude`**. This
> section traces when that changed. Cross-checked against git; the canonical
> write-up of the drift is `docs/retro/retro-provider-auth-isolation-regression-2026-06-05.md`.

### 9.1 The original design (isolated, decoupled from global)

`docs/specs/provider-auth-isolation.md` — design dated **2026-03-21** (src-tauri
era; first committed to this repo 2026-05-14 in #850):

- Each provider had its own AgentMux-owned dir:
  `~/.agentmux/instances/v{version}/auth/{provider}/` — **isolated from the
  user's personal `~/.claude`**, shared across agents *of that version*.
- **§4 invariant ("the prophecy"):** *"the auth check must validate the SAME dir
  the agent uses … otherwise the check passes using personal `~/.claude/` but the
  subprocess fails using the isolated empty dir."* It explicitly warned against
  touching the global login.

### 9.2 The drift (dated; commits verified in git)

| Date | PR / commit | What it introduced |
|------|-------------|--------------------|
| 2026-03-21 | `provider-auth-isolation.md` | **Isolated** per-provider auth, decoupled from `~/.claude`; §4 invariant |
| **2026-05-14** | **#850** `f836aae5` | First **global `~/.claude` fallback** — two-phase `CheckCliAuth` phase-1 reads the personal login (the §4 violation) |
| **2026-05-22** | **#983** `136f49fa` | **"seed Default identity bundle from ambient `~/.claude` on startup"** (`agentmux-srv/src/identity/migration.rs`). *This is the mechanism that makes today's `identity=default` agents resolve to the global `~/.claude`.* |
| 2026-05-24 | #1027 `87726ae5` | Channels re-rooted auth from version-shared → **per-channel** (multiplied the empty-dir hazard) |
| 2026-06-05 | — (retro) | The "validate-spin" regression surfaces; retro written |
| **2026-06-05** | **#1291** `d1ecdc3c` | **"provider auth in one shared, instance-independent dir"** — introduces `~/.agentmux/shared/providers/<provider>/` (the `.join("shared")` path) + `SPEC_PROVIDER_PINNED_AUTH` |
| ~2026-06-13+ | #1391 / #1396 / #1399 / #1403 … | **Cross-channel GLOBAL**: agents + auth + transcript shared across all channels/versions |

### 9.3 "Shared global auth" was actually two separate changes

1. **Coupling to the global `~/.claude`** — crept in via **#850 (2026-05-14)** as a
   check-fallback, then cemented by **#983 (2026-05-22)** when the Default identity
   bundle was seeded from / pointed at ambient `~/.claude`. This is why Nark
   (identity `default`) reads `~/.claude` today.
2. **The single shared AgentMux dir** (`~/.agentmux/shared/providers/<provider>/`)
   replacing per-instance isolation — **#1291 (2026-06-05)**, the post-retro
   structural fix, later generalized to fully global by the cross-channel work
   (#1383–#1403).

### 9.4 Nuance — directive vs. drift

The 2026-06-05 retro's own conclusion (lesson #3) was that auth **should** be a
shared *provider* property, quoting the user directive: *"every provider's
requirements is a pinned version, completely independent from the installed
instances."* But that directive pointed at an **AgentMux-owned** shared dir
(`~/.agentmux/providers/<provider>/`) — **not** the personal `~/.claude`. The
coupling to the global `~/.claude` (#850 / #983) is a **separate** drift that
predates and exceeds that directive; the retro explicitly labelled the earlier
"seed isolated creds from global" patch (#1283) a **symptom treatment**, not the
intended design.

**Net:** designed isolated-from-global → later directed shared-per-provider
(instance-independent) → but also quietly coupled to the global `~/.claude` along
the way. That last coupling is what produces the Nark-vs-Poal split in §2.
