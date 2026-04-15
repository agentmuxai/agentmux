# Report: AgentX in AgentMux reports "no git identity" — root cause and claw bridge fix

**Date:** 2026-04-14
**Severity:** Startup warning — every AgentX pane launch shows it; blocks
commits from inside the agent
**Affected:** `agentmux-srv/forge-seed.json`, AgentX environment on this
machine, the bridge between AgentMux's forge agent store and claw

---

## 1. Symptom

Launching the AgentX forge agent from the AgentMux agent picker:

1. AgentMux spawns Claude Code in a new pane with
   `working_directory = ~/.agentmux/agents/agentx`.
2. Claude Code's startup output includes:
   ```
   ⚠ No git identity configured. Set user.name and user.email
     with `git config --global ...` to enable commits.
   ```
3. Any tool call that touches git (`Bash git commit …`) fails because
   the host has no `user.name` / `user.email`.

This is **not** AgentMux's message — a `grep -r "no git identity"`
across `frontend/` and `agentmux-srv/src/` returns zero hits. The
warning is produced by Claude Code itself on startup when the working
directory is inside a git repo (or has a git parent) and no
`user.name`/`user.email` can be resolved.

---

## 2. Root cause — two stacked problems

### 2a. This host has no global git identity

```bash
$ git config --global --get user.name
(empty)
$ git config --global --get user.email
(empty)
```

Every terminal / agent / task started by the current user inherits
that empty state. On a fresh Windows dev machine this is normal —
the user hasn't run `git config --global user.name "..."` yet. Claude
Code flags it because every edit-then-commit workflow would silently
fail.

### 2b. AgentMux runs AgentX in the wrong directory

`agentmux-srv/forge-seed.json:12` sets:

```json
"working_directory": "~/.agentmux/agents/agentx"
```

On this machine that directory exists but contains only:

```
~/.agentmux/agents/agentx/
├── .mcp.json
├── ~/
└── specs/
```

**No `.git`, no CLAUDE.md, no repo state.** Meanwhile, the claw
deployment has already built the real AgentX workspace elsewhere:

```
~/.claw/agentx-workspace/
└── CLAUDE.md       ← the 'managed by claw' agent-identity doc
```

— and that's where claw expects AgentX to run (see
`a5af/claw:templates/host/CLAUDE.md` which reads
`You are **{{AGENT_DISPLAY}}**, running from ~/.claw/{{WORKSPACE}}`).

So AgentMux launches AgentX in an empty parking-lot directory that
has nothing to do with claw. Even if the user fixes the global git
identity, AgentX would still be operating from the wrong workspace —
no CLAUDE.md, no skills, no claw deployment state, no ability to run
`claw deploy`.

### 2c. Why the warning is structural, not incidental

Even with git identity fixed, the `~/.agentmux/agents/agentx/` path is
technically a subdirectory of `$HOME` — a non-git directory. Claude's
startup check walks up looking for a `.git` and finds none until it
hits a much higher ancestor (or the filesystem root, or whatever `$HOME`
happens to be part of). Depending on the walk, it may still warn about
missing identity because the closest useful git scope is absent.

The correct workspace — `~/.claw/agentx-workspace/` — is *also* not a
git repo (verified: `git config --get user.name` returns empty there
too). Claw's host template doesn't git-init the workspace. So moving
the working dir alone doesn't silence the warning; the global git
config still has to be right.

---

## 3. What claw *does* set up (and for whom)

Searching `a5af/claw` for `user.name` / `user.email`:

| Script | Target | Sets |
|---|---|---|
| `docker/lib/github-auth.sh:63` | **container** agents (agent1-5) via GitHub Apps | `user.name = ${agent}-workflow[bot]`, `user.email = <app-id>+${agent}-workflow[bot]@users.noreply.github.com` |
| `docker/lib/github-auth.sh:67` | **container** agents via PAT fallback | `user.name = ${Agent^}-asaf` (e.g. `Agent1-asaf`), `user.email = ${agent}@asaf.cc` |

**Host agents (AgentX, AgentY) have no claw-side git identity setup.**
The host templates under `templates/host/` include `CLAUDE.md`,
`STARTUP_PROMPT.md`, and `.mcp.json.template`, but nothing that runs
`git config`. Host agents inherit the user's global git identity —
which, on this machine, doesn't exist yet.

This is a gap on the claw side too, but fixing AgentMux to bridge
properly is the more urgent move because AgentMux already has a seed
manifest with a `working_directory` field that simply points at the
wrong place.

---

## 4. Fix — three layers

### Layer 1 (immediate, ~30 sec) — set the global git identity

On this host, run once:

```powershell
git config --global user.name "AgentX-asaf"
git config --global user.email "agentx@asaf.cc"
```

Mirrors claw's container-PAT naming (`Agent1-asaf`, `agent1@asaf.cc`).
Claude Code's startup warning goes away immediately. Every pane
launched afterwards — AgentMux's forge agents, hand-started Claude,
gh CLI — now has a valid commit identity.

**Caveat:** host AgentX and host AgentY will both commit under
`AgentX-asaf` unless AgentY's working directory carries a local
`.git/config` override. For a solo dev that's fine. For the multi-host
setup claw envisions, each host agent should have its own local git
config inside its workspace (see Layer 3).

### Layer 2 (short — point AgentMux at the right workspace, ~1h)

Update `agentmux-srv/forge-seed.json` to use claw's workspace paths
for host agents:

```json
{
  "id": "agentx",
  "name": "AgentX",
  ...
  "working_directory": "~/.claw/agentx-workspace",
  ...
},
{
  "id": "agenty",
  "name": "AgentY",
  ...
  "working_directory": "~/.claw/agenty-workspace",
  ...
}
```

Then:

- AgentX boots inside `~/.claw/agentx-workspace/` and immediately
  sees claw's managed `CLAUDE.md` with the correct agent identity
  prompt.
- Any `claw deploy` / `claw status` invocation from inside the pane
  runs from the directory claw expects.
- Skills, secrets, and context files that claw lays down land in a
  directory the agent is actually in.

**Compatibility note:** this only works for machines where the user
has already run claw's `bootstrap.ps1` and deployed the agentx
workspace. On a machine without claw, `~/.claw/agentx-workspace/`
doesn't exist and Claude Code starts in `$HOME` as fallback. The
fix for that is in Layer 3 — AgentMux should detect claw and offer
to bootstrap it.

### Layer 3 (medium — AgentMux ↔ claw bridge, ~4-6h)

Wire a formal bridge so AgentMux stops embedding duplicate agent
definitions and defers to claw as the system of record for host
agents. Possible shape:

1. **New module:** `agentmux-srv/src/backend/claw_bridge.rs`
   - Detects whether claw is installed: check for `~/.claw/claw.ps1`
     or `~/.claw/manifest.json`.
   - Reads claw's `~/.claw/manifest.json` to enumerate configured
     host agents (AgentX, AgentY, …) and their workspace paths.
   - On first run, if claw is present but the forge store has stale
     `agentx`/`agenty` entries pointing at `~/.agentmux/agents/...`,
     migrate them to point at the claw paths.

2. **Forge seed change:** mark host agents as "provided by claw"
   instead of hardcoding their working_directory. When claw is
   present, the agent picker shows "(claw)" next to the name. When
   not, they're hidden OR shown as "install claw to enable."

3. **Git identity probe:** when launching a host agent, run a
   pre-flight `git config --global user.name` + `user.email` check.
   If missing, pop a dialog offering to set them to the claw default
   pattern (`AgentX-asaf` / `agentx@asaf.cc`). User confirms once;
   AgentMux runs the `git config --global` calls.

4. **Per-workspace override:** for AgentY, also write
   `~/.claw/agenty-workspace/.git/config` (if the workspace is a git
   repo) with `user.name = AgentY-asaf` / `user.email = agenty@asaf.cc`
   so commits from that pane attribute correctly even though the
   global is AgentX-asaf.

None of step 3 or 4 is needed for Layer 1 to work — they're a polish
pass once the basic bridge is in place.

---

## 5. Proposed commit — smallest viable fix (Layer 1 + 2)

Two changes, one PR:

**`agentmux-srv/forge-seed.json`** — swap the two host agent
working_directory fields:

```diff
-      "working_directory": "~/.agentmux/agents/agentx",
+      "working_directory": "~/.claw/agentx-workspace",
```

```diff
-      "working_directory": "~/.agentmux/agents/agenty",
+      "working_directory": "~/.claw/agenty-workspace",
```

**Release note / README update** — add to `CLAUDE.md` or `BUILD.md`:

```markdown
## Host agent prerequisites

AgentX and AgentY are host-side forge agents that expect their
workspaces to exist under `~/.claw/`. Before launching them from
the AgentMux agent picker:

1. Install claw (one-liner from
   https://github.com/a5af/claw/blob/main/bootstrap.ps1):
   ```powershell
   irm https://raw.githubusercontent.com/a5af/claw/main/bootstrap.ps1 | iex
   ```
2. Deploy the host workspaces:
   ```powershell
   claw deploy agentx-workspace
   claw deploy agenty-workspace
   ```
3. Set git identity once (claw naming convention):
   ```powershell
   git config --global user.name "AgentX-asaf"
   git config --global user.email "agentx@asaf.cc"
   ```

On machines without claw installed, host forge agents will still
launch but will operate from `$HOME` and commits will fail until
git identity is configured.
```

**No migration needed.** The existing `~/.agentmux/agents/agentx/`
directory is essentially empty on real machines and can be left
alone as a safe fallback path. If a migration is desired, it's a
single `mv` on the backend during seeding.

**Risk:** low. Users without claw see the same "no workspace" state
they do today, just under a different path. Users with claw
immediately get the right workspace.

---

## 6. Open questions

1. **Should AgentMux ship a `claw install` wizard?** When a host
   agent is launched on a machine without claw, should the forge
   picker offer to run `bootstrap.ps1` instead of falling through to
   `$HOME`? — Leaning yes but as a follow-up. The Layer 1+2 change
   doesn't need it.
2. **Should the git identity probe on launch be blocking or
   advisory?** If AgentMux detects missing identity and pops a
   dialog, does it block launch until resolved or just warn and
   continue? — Advisory. Blocking would make the agent unusable for
   users who don't care about commits from that pane.
3. **Does claw want its host agents git-init'd during deploy?**
   i.e. should `claw deploy agentx-workspace` run `git init` in
   `~/.claw/agentx-workspace` and set a local `user.name`/`user.email`?
   That would make the "no git identity" warning go away without
   touching the user's global config. — Worth raising on the claw
   side as a separate issue. Out of scope here.
4. **Where does AgentZ (Gemini host) fit?** The forge-seed has
   `agentz` but there's no `~/.claw/agentz-workspace/` currently.
   Either add it to claw or drop AgentZ from the seed until claw
   catches up.

---

## 7. Verification after fix

1. Apply Layer 1 (`git config --global` commands).
2. Apply Layer 2 (forge-seed.json changes), rebuild, restart AgentMux.
3. Open the agent picker → launch AgentX.
4. Observe: Claude Code starts in `~/.claw/agentx-workspace/`, no
   "no git identity" warning, the managed claw `CLAUDE.md` is
   visible on first read.
5. Run `/runtime` in the composer to sanity-check slash commands
   still work post-restart.
6. Have AgentX attempt a trivial commit (e.g. `git init` + touch
   + commit a scratch file). Should succeed as `AgentX-asaf
   <agentx@asaf.cc>`.
