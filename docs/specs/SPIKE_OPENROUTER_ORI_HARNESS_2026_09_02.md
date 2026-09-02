# Spike: OpenRouter's Ori harness — does it change our integration story?

**Status:** proposed — findings verified by running Ori 0.13.0 locally; the §6
integration options have not been built.
**Date:** 2026-09-02
**Owner:** AgentY
**Related:** [`SPEC_ADD_PROVIDERS_QWEN_AIDER_2026_06_02.md`](./SPEC_ADD_PROVIDERS_QWEN_AIDER_2026_06_02.md)
(still `Status: Draft`) — its OpenRouter taxonomy is what this spike overturns,
and [`SPEC_PROVIDER_MODELS_EFFORT_GENERALIZATION_2026-06-14.md`](./SPEC_PROVIDER_MODELS_EFFORT_GENERALIZATION_2026-06-14.md)
(effort translation, §5).

---

## 0. Why this exists

The 2026-06 spec drew this taxonomy:

```
harness (Qwen Code | aider | ...)  ->  LLM gateway (OpenRouter / LiteLLM)  ->  model
```

and concluded OpenRouter is *only* a gateway, so using it means per-provider
env-var plumbing (`OPENAI_BASE_URL` + `OPENAI_API_KEY`). That is why the only
OpenRouter path in the product today is Qwen Code carrying
`supportedVendors: ["openrouter"]` (`catalog.ts:267`, `providers.rs:317`),
configured by hand.

**That premise no longer holds.** OpenRouter now ships harnesses of its own.
Everything below was verified by running the binary, not read off a webpage.

## 1. OpenRouter provides TWO kinds of harness

From `ori harness list` (machine JSON):

**(a) A first-party agent loop** — not a wrapper:

```json
{ "name": "ori", "origin": "builtIn", "default": true,
  "featureId": "@ori-runloop/agent-loop",
  "defaultModel": "anthropic/claude-fable-5.1" }
```

Invoked as `ori code` — "Run Ori as a local coding agent in the current
directory". A direct peer of Claude Code / Codex, not a router.

**(b) Launchers for 12 external CLIs.** `ori <agent>` runs the real binary from
PATH with OpenRouter credentials injected. It does **not** reimplement them
(DeepSeek Harness is the documented exception — Ori configures it instead).

Detected on this machine:

| Agent | installed | Already in our catalog? |
|---|---|---|
| claude, codex, opencode, hermes | yes | **claude, codex** |
| grok, omp, prime-agent, kilo, cline, pi, muse, dsh | no | **pi** |

Three overlap with providers we already ship.

## 2. Windows: native, with one real bug

`ori-windows-x64.exe` and `install.ps1` ship in every release (alongside
darwin/linux, arm64/x64, musl). No WSL needed. Checksum verified against
`SHA256SUMS` before running anything.

**BUG — workspace bootstrap fails when GNU tar shadows bsdtar.** First run
fetches a templates archive and extracts it with `tar`. With MSYS/Git-Bash tar
first on PATH:

```
tar (child): Cannot connect to C: resolve failed
ProjectInitError: Project initialization failed while fetching templates
```

GNU tar reads `C:\Users\...\.ori\global` as a remote `host:path`. Re-running
with `PATH=C:\Windows\System32;C:\Windows` — so Windows' bsdtar 3.8.4 wins —
bootstraps cleanly, creating `~/.ori/global` with `AGENTS.md`, `.agents/`,
`.claude/`.

This is **not** "Ori is broken on Windows". It is broken in a very common
Windows dev environment, and specifically in ours: our own `CLAUDE.md` tells
agents to put `Git\bin` on PATH for `task dev`. Any integration must sanitise
PATH for the child process or pre-create the workspace.

## 3. Flag passthrough: works, with a bounded exception

**Passthrough is real.** `ori claude --zzz-not-a-real-flag` produced
`error: unknown option '--zzz-not-a-real-flag'` — Claude Code's own
Commander.js error, from the real binary. Likewise `ori codex exec --json
--zzz-probe` produced codex's clap-style `unexpected argument '--zzz-probe'
found / Usage: codex exec`.

**What Ori consumes before the agent sees it:**

- Global: `--help/-h`, `--version/-v`, `--wizard`, `--completions`,
  `--log-level`, `--json/--agent`, `--human/--tty`
- Per-agent: `--model`, `--reasoning-effort`

**Demonstrated collision:** `ori claude --version` prints
`ori v0.13.0+c7b5cda`, **not** Claude Code's `2.1.112`. Any health or auth probe
built on `<cli> --version` silently reports Ori's version instead of the agent's.

Checked against our actual args in `catalog.ts`:

- **claude** — `-p --output-format stream-json --verbose
  --include-partial-messages --dangerously-skip-permissions`: **no collisions.**
- **codex** — `exec --json ...`: `--json` is also an Ori global. My probe did
  **not** isolate it, because codex errored on the deliberate bad flag either
  way. Treat this as **unverified risk**, not a confirmed break. It is the first
  thing to test if we wrap codex.

## 4. Auth: OAuth is not required

The open question was whether OAuth PKCE would fight our identity/auth-dir
model. It does not have to:

- `OPENROUTER_API_KEY` in the environment is a first-class credential path.
  Verified — Ori announces `Using the OpenRouter credential from the
  OPENROUTER_API_KEY environment variable.` on **stderr**.
- `ori login --with-key` accepts a piped key for fully non-interactive setup.
- Browser OAuth is one option of three, not the only one.

That maps onto our existing env-injected identity-bundle flow with no new
concepts.

**`ori auth` is a better auth check than what we use for Qwen.** It prints
machine-readable JSON (`{"ok": false, ..., "authenticated": false}`) and **exits
non-zero** when no credential resolves. Our Qwen entry uses the Gemini-inherited
`auth status` convention precisely because `--version` fails *open* — the
`catalog.ts` comment records that `checkcliauth` treats any non-JSON zero exit
as authenticated. `ori auth` fails closed by design.

## 5. `ori code` vs ProviderConfig — close, but not a drop-in

| `ori code` flag | Our field | Fits? |
|---|---|---|
| `--output jsonl` (runtime events + terminal result line) | `styledOutputFormat` | yes, but needs a NEW translator (§5.2) |
| `--model <openrouter-slug>` | model selection | yes |
| `--reasoning-effort max...none` | our effort generalization | yes, but stacks — see below |
| `--approvals self-drive` | the `--yolo` / `--dangerously-skip-permissions` slot | yes |
| `-p, --prompt <string>` | — | **NO — see below** |
| `--resume` / `--session <id>` | `resumeFlag` / `sessionIdField` | **not as written** |
| `--interactive, -i` | (TUI; not our path) | n/a |

**Two rows of an earlier revision of this table were wrong, and the corrections
are the most useful part of it** (caught in review on PR #2936).

**The prompt does not go in argv.** `launchArgs` is static, and
`SubprocessController` writes the user's message to the child's **stdin** —
`subprocess/mod.rs:75` ("The user's JSON message to write to stdin"), with
`subprocess/argv.rs` maintaining an explicit "stdin marker" position that
provider flags must not cross. Our `claude` entry exploits exactly that: it
passes a bare `-p` with no value, and Claude Code reads the prompt from stdin.
`ori code` documents `--prompt, -p string` as taking an argv value, so copying
the claude pattern would leave `-p` without its required argument.

**Answered in §5.1: it does not.** `--prompt-file` is the way, which makes this
a controller change rather than a catalog entry.

**`sessionIdField` cannot express `--session <id>`.** It only names the JSON
property to capture an id *from* (`agent_io.rs:244`, default `session_id`); the
resume strategy then appends `resumeFlag` followed by that captured id. Pairing
it with the boolean `--resume` would emit `--resume <id>`. The correct shape is
`resumeFlag: "--session"`, and **§5.2 identifies the property: `runId`**, nested
inside the `runtime.event` envelope rather than at the top level.

`--reasoning-effort` overlaps `SPEC_PROVIDER_MODELS_EFFORT_GENERALIZATION_2026-06-14.md`:
Ori also translates effort into "the harness's native mechanism". Two
translation layers stacked is a decision to make deliberately, not to inherit.

### 5.1 The stdin question, answered: no

`ori code -p` with **no value** does not read stdin — it prints help, because
`-p` requires an argv value. So the claude pattern cannot be copied.

`--prompt-file <path>` **does** work, verified end to end: argument parsing
succeeds and a turn starts, with `"promptLength": 7` and
`"prompt": "say hi\n"` in the emitted events. For option (b) that means a temp
file per turn, written where `SubprocessController` currently writes stdin.
Cheap, but it is a controller change rather than a catalog entry.

### 5.2 The `--output jsonl` schema, captured

Obtained by driving a real turn (it fails at the model call on a dummy key, but
the framing is emitted first). Every line is a nested envelope:

```json
{"event": { ... , "type": "audit.event" | "runtime.event" }, "kind": "event"}
```

Two families:

- **`audit.event`** — `.event.audit` with `auditId`, `commandId`, `createdAt`,
  `level`, `message`, `name` (`command.received`, `command.failed`), and a
  `detail` carrying `cwd`, `model`, `promptLength`, `type: agent.invoke`.
- **`runtime.event`** — `.event.event` with `createdAt`, `eventId`, `harness`,
  **`runId`**, **`turnId`**, `payload`, and `type`. Observed types:
  `run.started`, `turn.started`, `runtime.error`. `eventId` is `<runId>:<seq>`.

**This answers the `sessionIdField` question from §5:** the id to capture is
`runId` (with `turnId` for per-turn correlation), carried on `runtime.event`
lines — not a top-level `session_id`. Because it is nested two levels deep
inside a discriminated envelope, **this needs a new translator**; neither
`claude-json` nor `gemini-json` will read it.

### 5.3 `ori code` needs `bun` on Windows — currently a blocker

Past the tar issue, the same run surfaced more:

```
Installing dependencies in C:\Users\asafe\.ori\global...
Could not refresh global workspace dependencies: NotFound: ChildProcess.spawn (bun install --silent)
WARN: feature boot degraded; feature "dashboard" disabled ... Could not resolve: "react-dom/server"
Runtime server error while swapping code skill wrapper: Unknown: FileSystem.rename (...)
{"failure":{"code":"ORI_RUNTIME_INVOKE_FAILED", "kind":"internal", "stage":"runtime"}}
```

The global workspace installs its own dependencies with **`bun`**, which is not
bundled and was absent here, so feature boot degrades and the invocation fails
before reaching the model. `harness` also reports `"unknown"` in the events,
consistent with the degraded boot.

So option (b) on Windows currently requires `bun` on PATH **and** bsdtar ahead
of GNU tar. Neither is exotic, but both are environment preconditions we would
be taking on, and the failure mode when they are missing is an internal runtime
error rather than a clear message.

## 6. Options

Two independent options; (a) is cheap, (b) is the interesting one.

**(a) Wrap existing providers.** `cliCommand: "ori"`, prefixing the agent name.
Cheap, but only buys OpenRouter billing and guardrails for CLIs users can
already run, and inherits both the `--version` collision and the tar/PATH bug.

"Translators unchanged" holds **for claude only**, and an earlier revision of
this section overclaimed it for codex. Our codex provider feeds the
`codex-json` translator from `exec --json`, and §3 records that `--json` is
also an Ori global whose ownership the probe did not resolve. If Ori consumes
it, codex emits human-readable output and **every wrapped turn silently
bypasses the translator** — output would degrade rather than error, which is
the worst failure shape. Wrapping codex is therefore gated on resolving that
one flag; wrapping claude is not.

**(b) Add `ori code` as a provider in its own right.** This is what the 2026-06
spec could not have proposed: a real harness with headless prompt, JSONL
streaming, session resume, and any OpenRouter model behind one credential —
much closer to "OpenRouter as a model option" than the gateway framing allows.

**Now costed rather than hoped at**, after §§5.1-5.3. It is not a catalog entry;
it is three pieces of work:

1. A **new translator** for the nested `runtime.event` envelope, capturing
   `runId` as the session id (§5.2).
2. A **controller change** to write the prompt to a temp file for
   `--prompt-file`, since `-p` will not take stdin (§5.1).
3. **Environment preconditions** — `bun` on PATH and bsdtar ahead of GNU tar —
   or `ori code` fails with an internal runtime error (§5.3).

None of that is large, and none of it is the "drop in a ProviderConfig" the
first draft of this table implied. Worth doing if OpenRouter-as-a-model-option
is a goal in itself; not worth doing incidentally.

**Suggested order:** (a) for claude only — the cheapest real thing — then
resolve codex's `--json` ownership, then (b) if the model-option goal stands.

## 7. Operational notes

- **Pin the version.** `cli-0.13.0-c7b5cda` was published 2026-09-02, the same
  day as this spike. Fast-moving; treat it like `CLAUDE_CODE_VERSION` is treated
  in reagent's `Dockerfile.lambda`.
- **Telemetry is on by default** ("anonymous... never records your prompts or
  credentials"); `ORI_TELEMETRY=0` disables it. Decide explicitly rather than
  shipping the default.
- **stdout/stderr are cleanly separated.** `--version` puts only
  `@ori-runtime/cli 0.13.0+c7b5cda` on stdout; credential notices go to stderr.
  `--json` guarantees "stdout carries exactly one JSON document".
- **Ori auto-installs missing agent CLIs on first use** — worth knowing before
  it installs something into a user's environment unprompted.

## 8. Reproduce

```bash
TAG=cli-0.13.0-c7b5cda
BASE=https://github.com/OpenRouterLabs/ori-releases/releases/download/$TAG
curl -fsSLO $BASE/ori-windows-x64.exe
curl -fsSLO $BASE/SHA256SUMS
sha256sum -c --ignore-missing SHA256SUMS

./ori-windows-x64.exe harness list      # both harness kinds
./ori-windows-x64.exe auth              # exits non-zero, machine JSON
./ori-windows-x64.exe claude --version  # prints ORI's version - the collision
OPENROUTER_API_KEY=dummy ./ori-windows-x64.exe claude --zzz  # claude's own error
```
