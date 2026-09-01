# Report — what `CLAUDE_CONFIG_DIR` actually isolates, measured

**Date:** 2026-09-01
**Author:** Manoz
**Status:** Complete. Live experiments; every claim below is measured, not inferred.
**Context:** `SPEC_ISOLATE_HOST_CLAUDE_MD_2026_08_31.md` shipped a fix
(PR #2854) on the *inferred* premise that an isolated `CLAUDE_CONFIG_DIR`
with no `CLAUDE.md` falls back to the operator's personal
`~/.claude/CLAUDE.md`. The operator then asked the sharper question — *"is
the global claude completely isolated?"* — which that spec could not answer,
because it had never tested the premise or the other surfaces. This report
answers it.

## 0. Summary

| Surface | Isolated by `CLAUDE_CONFIG_DIR` alone? | Evidence |
|---|---|---|
| `CLAUDE.md` (user memory) | **NO — leaks** | §2, three-arm experiment |
| `settings.json` (`env`, hooks, permissions) | **YES** | §3 |
| `skills/` | **YES** | §4 |
| `.claude.json` (MCP servers) | N/A on this machine — none defined | §5 |
| `plugins/` | N/A — sole plugin is project-scoped to `C:\Systems` | §5 |
| Other providers (`AGENTS.md`, `GEMINI.md`, …) | **UNTESTED** — no host file exists to leak today | §6 |

**Net: the `CLAUDE.md` leak was real, is the only confirmed one, and the
shipped fix closes it.** The rest of `~/.claude` was already isolated —
which is *why* the leak was easy to miss: the boundary works for
credentials, settings and skills, so nothing else pointed at it.

## 1. Method, and one methodological trap worth recording

Each surface gets a three-arm test, all from a working directory whose
entire parent chain is verified free of `CLAUDE.md` (otherwise project-level
discovery confounds the result):

1. **Control** — isolated config dir *without* the file under test. Does the
   host's version leak in?
2. **Treatment** — the same dir *with* the file. Is the leak closed?
3. **Sensitivity** — a distinctive sentinel in the config dir's own copy.
   Proves the instrument can actually see this surface at all.

**Arm 3 is not optional.** The first pass of this investigation ran arms 1
and 2 only, phrased as *"do your system instructions contain 'Global Claude
Code Rules'?"*, and got `NO` from every arm — which reads as "no leak,
fix unnecessary." That conclusion would have been wrong. Memory files arrive
as a user-turn reminder, not as literal system instructions, so the model was
answering the question as asked, truthfully and uselessly. Re-phrased to
*"quote the first markdown heading of any user-level instructions in your
context"*, the same arms immediately separated. **A null result from an
uncalibrated instrument is not evidence of absence** — anyone re-running this
should keep arm 3.

## 2. `CLAUDE.md` — leaks (the fix is load-bearing)

Prompt: *"Quote the first markdown heading of any user-level or global
instructions in your context. Reply with just that heading line, or NONE."*

| Arm | `CLAUDE_CONFIG_DIR` contents | Result |
|---|---|---|
| Control | `.credentials.json` only | **`# Global Claude Code Rules`** ← host file |
| Treatment | `+ CLAUDE.md` (AgentMux placeholder) | **`NONE`** |
| Sensitivity | `+ CLAUDE.md` containing `ZORBLAX_SENTINEL_9931` | returned the sentinel |

The sensitivity arm also establishes the *mechanism*:
`$CLAUDE_CONFIG_DIR/CLAUDE.md` **is** the user-memory location and **is**
read. So the failure mode is specifically *fallback on absence* — an empty
isolated dir reaches past itself to `~/.claude/CLAUDE.md`. Seeding any file
at that path stops it.

## 3. `settings.json` — isolated

The highest-stakes surface: this machine's host `settings.json` carries
`env.BASH_ENV`, a `PostToolUse` hook shelling out to an unrelated repo
(`/c/Systems/agentmux`), and `skipDangerousModePermissionPrompt: true`. A
fallback here would be materially worse than leaked instructions.

| Arm | Config dir | Observable (`echo $BASH_ENV` / `$AMX_SENTINEL` via Bash) | Result |
|---|---|---|---|
| Control | no `settings.json` | `BASH_ENV` | **empty** — host `env` did not leak |
| Sensitivity | `settings.json` with `env.AMX_SENTINEL=TRIPWIRE_7742` | `AMX_SENTINEL` | **`TRIPWIRE_7742`** |

Config-dir settings are read and applied; absence does **not** fall back.
`settings.local.json` is loaded by the same config-dir resolution and is
assumed to behave identically — *inferred by analogy, not directly measured*,
because the host copy contains no observable (`enableAllProjectMcpServers`,
`enabledMcpjsonServers: ["agentmux"]`) that can be detected without a
purpose-built MCP fixture.

## 4. `skills/` — isolated

Host has three custom skills (`build`, `deploy`, `pr`).

| Arm | Config dir | Skills listed |
|---|---|---|
| Control | no `skills/` | built-ins only — **no `build`/`deploy`/`pr`** |
| Sensitivity | `+ skills/zorbtest/SKILL.md` | **`zorbtest`** appeared, first in the list |

## 5. Surfaces with nothing to leak here

- **`.claude.json`** — parsed: zero top-level `mcpServers`, zero project-scoped
  ones. No MCP definitions exist to leak on this machine.
- **`plugins/`** — one installed plugin (`rust-analyzer-lsp`), `scope: project`,
  `projectPath: C:\Systems`. AgentMux agents do not run under that path.

Both are "no exposure *today*" rather than "isolated by construction" — a
user who later defines a global MCP server would need this re-checked.

## 6. Other providers — open, but not currently exposed

The same class of bug could exist for any provider with a user-level
instructions convention: Codex (`AGENTS.md`), Gemini (`GEMINI.md`), Qwen
(`QWEN.md`). Host config dirs **do** exist for `.codex`, `.gemini`, `.kimi`,
`.openclaw` — but **none of them contains its instructions file**, so there is
nothing to leak right now:

```
absent: ~/.codex/AGENTS.md     absent: ~/.gemini/GEMINI.md
absent: ~/.qwen/QWEN.md        absent: ~/.openclaw/{AGENTS,CLAUDE}.md
```

The fix stays deliberately `claude`-only. Extending it generically was
considered and **rejected**: `ProviderConfig::startup_instructions_filename`
describes the file AgentMux writes into the agent's *working directory*
(project level), which is not necessarily the same as that provider's
*user-config-level* filename. Reusing it for the config-dir placeholder would
conflate two different concepts on a guess. Closing this properly means
running §1's three-arm method per provider, which needs each CLI installed and
authenticated (`codex`, `qwen`, `openclaw` are not installed here).

## 7. Coverage on disk

Seeding happens at spawn time, so a dir is protected the moment it is
actually used. At time of writing **1 of 37** isolated `claude` config dirs on
this machine carries the placeholder — the rest are overwhelmingly dead
per-build channels that will never be spawned into again. The number is
expected to stay low and is not itself a defect.

The real exposure window is different: an agent spawned from a build
**older than v0.55.30** still runs the pre-fix code and will leak regardless
of what is on disk.

## 8. Hardening applied after this investigation

The default spawn path (`agent_open.rs`) originally did `create_dir_all` and
the isolation seed as two independent statements, with **no test covering the
seed call at all** — deleting that one line would have silently reopened the
leak with a green test suite. Both are now fused into
`providers::prepare_provider_auth_dir()`, so a caller cannot obtain a usable
auth dir without its isolation guarantees; removing the isolation now means
removing the directory creation too, which fails loudly. Four tests cover it
(creation+seed together, non-claude no-op, idempotence/never-clobber, empty-path
rejection).
