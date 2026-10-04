# REPORT — Out-of-workspace clone audit

**Date:** 2026-09-05
**Author:** Loap #2 @ claudius
**Scope:** This machine (`claudius`). Every git clone/worktree of an
AgentMux-org repo, checked against the rule that an agent must work from a
clone **inside its own workspace**.
**Status:** implemented — both §2 and §3 cleanup **executed** 2026-09-05 after
operator sign-off. `C:\Systems\` now holds exactly one git repo — a
third-party project unrelated to AgentMux (§3.2) — and zero
out-of-workspace clones of any AgentMux-org repo. The two §4 blocker
documents were resolved by decision rather than rescue (operator confirmed
both stale); everything else of value was preserved first (§3.1, §4).

---

## 0. The rule, and how it is expressed on disk

Agent workspaces live at `~/.agentmux/agents/<agent-id>/`. The established
convention — followed by 23 of the agents on this machine — is that each
agent's repo clones sit **inside** that directory:

```
~/.agentmux/agents/agent2-0630f/agentmux/          ✅ in-workspace
~/.agentmux/agents/korp-0620g/agentmux-docs/       ✅
```

Anything under `C:\Systems\` is **outside** every workspace and therefore
non-compliant, regardless of who created it.

## 1. Verdict

**No agent is currently *running* from an out-of-workspace clone.** Nothing
executing on this machine has a binary path under `C:\Systems\`, and no
agent's `CLAUDE.md`, `.claude/`, or `.mcp.json` references a `C:\Systems\`
path — so no agent is *configured* to work there either. The exposure is
entirely from **abandoned working state left behind** by past sessions,
mine included.

Corrected during this audit: **my own workspace (`loap-2-0822g`) had no
clone at all**, and every branch I produced this session was built in
`C:\Systems\` worktrees. A compliant clone now exists at
`~/.agentmux/agents/loap-2-0822g/agentmux/`, and this report was written
from it.

## 2. Out-of-workspace AgentMux clones and worktrees (`C:\Systems\`)

`C:\Systems\agentmux` is a full clone; the three `agentmux-*` entries below
it are **worktrees of that clone**, so removing the base clone invalidates
all of them. They must be treated as one unit.

| Path | Branch | Last commit | PR | Uncommitted |
|---|---|---|---|---|
| `C:\Systems\agentmux` (base clone) | `agentc/fix-low-memory-resume-button` | 2026-06-09, AgentC | #1316 **merged** | **12 files** |
| `C:\Systems\agentmux-codex-jsonl-spec` | `codex/spec-jsonl-contract` | 2026-08-09, AgentC | #2476 **merged** | 0 |
| `C:\Systems\agentmux-main-inspect` | `fix/composer-strip-left-right-balance` | 2026-08-25, Loap #2 | #2808 **merged** | **2 files** |
| `C:\Systems\agentmux-mcp-window-tools` | `feat/agent-app-api-window-discovery` | 2026-08-25, Loap #2 | #2810 **merged** | 0 |
| `C:\Systems\agentmux-wt-help-restore` | `agentc/muxbus-wire-namespace` | — (**prunable**, dir already gone) | #1736 **merged** | n/a |

Every branch's PR is merged. Each worktree reports "unmerged commits vs
`origin/main`", but that is a **squash-merge artifact** — the original
branch commits are not ancestors of `main` even though their content is.
Content was verified present on `main` file-by-file; no committed work is
at risk.

Three worktrees created during this session
(`agentmux-cred-broker` #2824, `agentmux-login-flow` #2971,
`agentmux-window-snap` #2986) were already removed after their PRs merged
and their content was verified on `main`.

## 3. Other out-of-workspace clones (`C:\Systems\`)

Not AgentMux-repo worktrees, but same non-compliance: eight further clones
of AgentMux-org repositories (public and private), plus one third-party
project (`SunoHarvester`). None was in active use by a running agent.

**All eight AgentMux-org clones were removed 2026-09-05** after per-repo
checks. `SunoHarvester` was **kept** — see §3.2.

### 3.1 What the checks found

- **One clone held work that existed only on this disk** — commits ahead
  of its own remote branch, hidden by a stale `origin/…` ref until an
  explicit `git fetch`. It was pushed to a salvage branch on its remote and
  verified by SHA before anything was deleted; its owner should triage it.
- **One clone contained a CLI some agents use.** Removing it was safe only
  after confirming the command on `PATH` is an independently installed
  package, not a link into `C:\Systems`, and that it worked before and
  after removal.
- **`agentmux-agy`** — 11 genuinely-unique uncommitted lines
  (`harness_engine` / `model_vendor` struct fields; confirmed absent from
  `main`, which has only the unrelated `model_vendor_base_url`). Saved as
  a patch alongside AgentC's (§4).
- **The rest** were clean, or carried only superseded generated output,
  submodule-pointer bumps, or line-ending noise. Nothing to preserve.

### 3.2 `SunoHarvester` — deliberately kept

Its remote is **`hartmark/SunoHarvester`** — a third-party project, not an
AgentMux-org repo and not an agent clone. It carries 10 untracked Python
scripts (real, unpushed personal work). It is out of scope for a rule about
where *agents* work, and deleting it would have destroyed unrelated
content. Left exactly as found.

## 4. BLOCKERS — resolved by decision (2026-09-05)

> **Outcome:** the operator reviewed both documents below and judged them
> stale and not worth keeping ("those docs are old, not needed"). They were
> **discarded, not rescued** — neither was committed to `main` before
> `C:\Systems\agentmux` was deleted, so both are gone. Recorded here so the
> loss is deliberate and traceable rather than silent.
>
> AgentC's 9 modified tracked files (below) were **preserved as a
> patch** rather than discarded, since they are not the operator's or mine
> to write off — see that sub-section.

Two documents existed **only** in an out-of-workspace clone and were **not
on `main`**. Deleting those clones destroyed them.

1. **`RESEARCH_CODEX_CREDENTIAL_ISOLATED_BROWSING_2026_08_26.md`** (discarded, never on `main`)
   — in `C:\Systems\agentmux`, untracked. The research report behind the
   credential-isolated browsing feature that shipped as PR #2824; the PR
   carried the code and spec but not this document.
   *Mine.* Written there before I discovered that checkout was stale.
2. **`SPEC_AGENT_COMPOSER_STRIP_THREE_ZONE_RESPONSIVE_2026_08_24.md`** (discarded, never on `main`)
   — in `C:\Systems\agentmux`, untracked. The composer-strip responsive
   design spec from the work handed off as PR #2808 / issue #2809.
   *Mine.*

Verified as **already on `main`** and therefore safe to lose:
`SPEC_CODEX_JSONL_CONTRACT_2026_08_08.md`,
`REPORT_AGENT_SCREENSHOT_WINDOW_CONTROL_BLOCKERS_2026_08_24.md`,
`SPEC_AGENT_APP_API_WINDOW_CONTROL_ROBUSTNESS_2026_08_24.md`.

### Judgement call, flagged rather than made

`C:\Systems\agentmux` also holds **9 modified tracked files** (+32/−37
lines) across the OAuth/identity area — `providers.rs`,
`auth_patterns.rs`, `migration.rs`, `resolver.rs`, `cli_handlers.rs`,
`identity_handlers.rs`, `SPEC_OAUTH_IDENTITY_BUNDLES_2026_05_22.md`, and
the provider catalog + its test.

They sit on a branch last committed **2026-06-09**, ~1,400 commits behind
`main`, in an area that has since been substantially rewritten (the
per-channel auth work of `ANALYSIS_PER_CHANNEL_AUTH_BYPASSES_2026_08_31.md`
and PR #2878 touched several of these exact files). They are almost
certainly abandoned scratch work, and rebasing them onto today's `main`
would likely conflict throughout.

They are **not mine** (AgentC's branch), so I did not judge them
disposable even under a general "clear it" instruction. **Captured as a
patch before deletion:**

```
~/.agentmux/agents/loap-2-0822g/salvage/
    agentc-june-identity-wip-2026-09-05.patch          (251 lines, `git diff`)
    agentc-june-identity-wip-2026-09-05.filelist
    agy-harness-model-decoupling-wip-2026-09-05.patch  (22 lines, from §3.1)
    (one further file list)
```

Reapply with `git apply` from a repo root if anyone ever wants it. Expect
conflicts — it is ~1,400 commits stale against code that has since been
rewritten. If nobody claims it, deleting the patch is a no-questions
cleanup; the point was only to not make that call silently on someone
else's behalf.

## 5. What was executed (2026-09-05)

1. Preserved everything unique **before** deleting anything: the
   disk-only commits pushed to a salvage branch and SHA-verified against
   the remote;
   AgentC's and AgentY's uncommitted deltas saved as patches.
2. `git worktree remove` ×3 → `git worktree prune` (cleared the dangling
   `agentmux-wt-help-restore`) → deleted `C:\Systems\agentmux`.
3. Cleared the tooling clone only after proving the CLI on `PATH` is an
   independent install, verified working before *and* after removal (§3.1).
4. Deleted the remaining seven AgentMux-org clones.
5. Left `SunoHarvester` untouched (§3.2).

**Nothing was migrated, by design.** Every branch involved was either
merged or preserved on a remote, so a fresh in-workspace clone is strictly
better than relocating a stale directory. My own compliant clone at
`~/.agentmux/agents/loap-2-0822g/agentmux/` was created that way — a plain
`git clone`, ~6 seconds.

### Residual items someone else should close out

- The salvage branch from §3.1 (AgentY's commits). Triage or delete.
- `~/.agentmux/agents/loap-2-0822g/salvage/` → two patches (AgentC's June
  identity WIP, AgentY's harness/model fields) + two file lists. Both are
  stale against current `main`; deleting them is a no-questions cleanup
  once their owners don't want them.

## 6. Why this recurred, and the cheapest guard

There is no enforcement anywhere — no hook, no gate, nothing in
`agents/CLAUDE.md` that states the in-workspace rule. `C:\Systems\agentmux`
existing and being *convenient* is the whole reason it kept being used; I
reached for it twice this session before checking its branch, and both
times it was 1,400+ commits stale, which silently invalidated the
exploration built on it until I caught it.

Two cheap, durable guards, in order of value:

- **State the rule in `~/.agentmux/agents/CLAUDE.md`** — one line naming
  `~/.agentmux/agents/<agent-id>/<repo>/` as the only sanctioned location,
  plus the reason (a shared checkout drifts, and its staleness is
  invisible until it has already misled you).
- **Remove `C:\Systems\agentmux` once §4 clears.** The path's mere
  existence is the attractor; deleting it removes the failure mode far
  more reliably than documentation does.
