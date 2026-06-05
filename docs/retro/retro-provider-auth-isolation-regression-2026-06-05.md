# Retro: How Provider Auth Isolation Regressed Into the "Validate-Spin" Bug

**Date:** 2026-06-05
**Author:** AgentA
**Trigger:** A freshly-created agent never authenticates — UI stuck "not
authenticated", CPU pinned ~100%, manual login times out at 15 s, "already
logged in but it won't take." User: *"but this was already working before … we
had set this up."* They're right. This is a **regression**, and the original
design **predicted the exact failure**.

---

## TL;DR

Provider auth was designed (2026-03-21) to live in **one version-shared dir**
(`~/.agentmux/instances/v{version}/auth/{provider}/`) with a single rule: *the
auth check must validate the SAME dir the agent uses.* Two later changes broke
that rule:

1. **#850** added a two-phase auth check whose phase 1 **falls back to the
   global `~/.claude`** — the precise "check global / run isolated" split the
   spec's §4 warned against.
2. **#1027 (channels)** re-rooted config (and with it, auth) from a
   version-shared dir to a **per-channel / per-data-dir** dir.

Per-instance empty auth dir (from #1027) + global-fallback check (from #850) =
"credentials found, but loggedIn:false, forever" → a 2 s CLI re-spawn spin. The
fix is to return to the spec's own *"Future: shared"* mode, generalized to the
user's directive: **every provider is a pinned unit, independent of installed
instances.**

---

## What we set up (the design that worked)

`docs/specs/provider-auth-isolation.md` — **2026-03-21**:

- Auth dir = `~/.agentmux/instances/v{version}/auth/{provider}/`, keyed on the
  **AgentMux version** and **shared across all agents/data-dirs of that
  version** (§"Shared vs Per-Agent Auth"). Log in once per version → every agent
  uses it silently.
- **§4 "Auth Check — Use Isolated Dir" (the prophecy):**
  > The `CheckCliAuth` command must pass the auth dir env var when running
  > `claude auth status --json`. **Otherwise the check passes (using personal
  > `~/.claude/`) but the subprocess fails (using the isolated empty dir).**

That single invariant — *check the dir you run in* — is the whole ballgame. It
was written down. It was then violated from two directions.

---

## Evolution timeline

| Date | PR / Spec | Change | Effect on auth dir |
|------|-----------|--------|--------------------|
| 2026-03-21 | `provider-auth-isolation.md` | Design: version-shared auth at `instances/v{ver}/auth/{provider}`; §4 invariant | **Intended: shared per version** |
| 2026-05-14 | #850 `feat(auth): AuthFlowController (PR B-2)` | Implements `auth_config_dir_env_var` + the **two-phase** `CheckCliAuth`: phase 1 "creds exist?" falls back to **global `~/.claude`**, phase 2 validates the **isolated** `CLAUDE_CONFIG_DIR` via `claude auth status --json` | Check can now pass on global while the agent runs isolated — **violates §4** |
| 2026-05-22 | #978–#982 OAuth Identity Bundles | Per-**identity** isolation (`identity_dir(bundle)/…`, per-bundle `CLAUDE_CONFIG_DIR` override) for deliberate multi-account | Adds a legitimate second isolation layer (keep this) |
| 2026-05-24 | #1027 `feat(common): channels` (`SPEC_DATA_CHANNELS`) | Data/config move to **per-channel** dirs (`~/.agentmux/channels/<channel>/config/…`); dev mode → `~/.agentmux/dev/<branch>/…` | **Auth root silently moves from version-shared → per-channel / per-data-dir** |
| 2026-06-05 | — | Fresh dev-branch / channel + a globally-authed user: phase 1 finds global creds, phase 2 validates the empty isolated dir → `loggedIn:false` forever | **The spin surfaces** |

---

## Root causes (two, compounding)

1. **The global fallback re-introduced the §4 hazard (#850).** To avoid a
   *false-positive* (a file-only check passing on stale/expired tokens — see
   `SPEC_AUTH_CHECK_FALSE_POSITIVE`), phase 1 was made to accept creds from
   *either* the isolated dir *or* global `~/.claude`. But phase 2 still validates
   only the isolated dir. That is exactly "check global / run isolated" — the
   thing §4 said never to do. On a populated dir it's harmless; on an **empty**
   one it loops.

2. **Channels re-rooted auth per-instance (#1027).** The 2026-03-21 design put
   auth in a **version-shared** dir. Channels keyed `config_dir` (and therefore
   `config_dir/auth/<provider>`) on the **channel / data-dir**. Every dev branch,
   every portable channel, every `--fresh` build now starts with its **own empty
   auth dir** — multiplying the §4 hazard from "once per version" to "once per
   channel/branch/build." This is the same root as the recurring "where did my
   agents go?" confusion: per-data-dir isolation makes fresh instances look
   wiped. (See `feedback_dev_instance_empty_session_panic`.)

Neither change is "wrong" in isolation — the false-positive fix is real, channels
are a good model. They **combined** into a non-recoverable state because nothing
re-checked §4's invariant when the auth root moved.

---

## Symptom cluster (all one bug)

- `CheckCliAuth` every ~2 s, each "credentials found, validating via CLI", never
  settling → constant CLI re-spawn = **CPU ~100%**.
- `run_cli_login` PTY **15 s timeout** (can't write the isolated dir fast enough
  / OAuth never completes against an empty dir).
- "Trying to login but it says already logged in" (global is logged in; the
  isolated dir the agent uses is not).

Full diagnosis: `reference_claude_auth_validate_spin`.

---

## Lessons

1. **A written invariant is worthless if nothing re-checks it when the ground
   moves.** §4 ("check the dir you run in") was correct and explicit. It was
   broken twice — once by a feature that *needed* a fallback (#850), once by a
   feature that *moved the dir* (#1027). A test that asserts "the dir CheckCliAuth
   validates == the dir the agent's `CLAUDE_CONFIG_DIR` points at" would have
   failed at both PRs.
2. **"Found" and "logged in" are different states; never let one satisfy a poll
   that waits on the other.** The frontend polled on "logged in" while the
   backend answered "found-somewhere" → an unrecoverable spin. (cf. the reducer
   principle: every state needs a forward edge.)
3. **Per-instance isolation of a thing the user owns globally is an
   anti-pattern.** Auth, like the CLI binary, is a *provider* property, not an
   *instance* property. The CLI was already version-shared; auth drifted to
   per-instance and nobody noticed until channels multiplied it.

---

## Fix direction (where this goes)

The user's directive — **"every provider's requirements is a pinned version,
completely independent from the installed instances"** — is precisely the spec's
own *"Future: Version-to-Version Auth Migration → shared"* mode (§"Future"),
generalized one step further (independent of *version* too, not just data-dir):

- A provider = a **pinned** unit (CLI + config + auth) at a location independent
  of every channel / data-dir / AgentMux version (e.g.
  `~/.agentmux/providers/<provider>/`). Instances **reference** it.
- The empty-per-instance dir that the §4 hazard needs **cannot exist** → the spin
  is impossible by construction, and the global-vs-isolated dilemma dissolves.
- The per-**identity** bundle layer (#978–#982) stays for deliberate
  multi-account — that isolation is intentional and explicit.
- Pin `pinned_version` (claude is `"latest"` today — not actually pinned).

Tracked in `project_provider_pinned_independent`. The earlier "seed isolated
creds from global" patch (PR #1283, closed) treated the symptom; this retro is
why we're doing the structural fix instead.
