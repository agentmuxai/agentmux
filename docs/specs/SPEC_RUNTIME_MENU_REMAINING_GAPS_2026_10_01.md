# SPEC: Finishing the runtime menu — approval prompts, a remembered runtime, the resolved model, and install cleanup

**Date:** 2026-10-01
**Status:** proposed — nothing here is built, **but §7's harness has since been used and its results are recorded there**
(2026-10-01): several assumptions below are now observed facts, and one claim (Plan mode is read-only) was observed to be
false and has been corrected. The four items below are what is left after the runtime-menu work in
`docs/reports/REPORT_AGENT_RUNTIME_BINDINGS_2026_09_30.md` §8 (#4098 … #4171). Each states what exists, what is
missing, a proposal, how to test it, and what is still unknown.
**Author:** Agento
**Owner decisions needed:** §2.7 (what "Default" mode should do), §5.8 (how aggressive pruning may be), §3.7 (whether
a remembered runtime should override an agent definition's own model).
**Related:** `SPEC_DECISION_PROMPT_2026_04_24.md`, `SPEC_AGENT_CONTROL_PROTOCOL_2026_06_15.md` §5 Phase 2,
`REPORT_AGENT_RUNTIME_STATE_RECONCILIATION_2026_09_30.md` (the readback design),
`SPEC_AGENT_OPEN_LATENCY_2026_09_27.md` (the shared CLI install dir).

---

## 1. Why this exists

The runtime menu (mode · model · effort) now shows what the agent runs, and says so when it doesn't. What it cannot yet do:

| # | Item | One line |
|---|---|---|
| 1 | Approval prompts | The Mode options are honest about not prompting; making them *actually* prompt is the real feature. |
| 2 | Remembered runtime | Continue / Reattach / New session start from the defaults because nothing stores the last runtime. |
| 3 | Resolved model | The menu shows the alias it asked for (`sonnet`), never the model the CLI resolved it to. |
| 4 | Install cleanup | Old CLI installs accumulate forever (~200 MB per pin bump per provider; 8.3 GB of legacy folders on one machine). |

Plus smaller items (§6) and a way to verify the CLI behaviours this work has *assumed* (§7).

**Ground rule for all of it:** the menu must never claim more than is true. Where a change can't be verified without a
running build or a third-party CLI, this spec says so and says how to verify it.

---

## 2. Item 1 — Make the permission modes real

### 2.1 What is true today

A persistent Claude agent runs on the control protocol (`--permission-prompt-tool stdio`). The CLI sends a
`can_use_tool` request for each tool call its mode wants approval for, and srv answers **every one** of them itself
(`handle_control_frame`, `crates/srv/src/backend/blockcontroller/persistent/input.rs`):

- `AskUserQuestion` is parked and shown to the user (works).
- Every other tool, **including the request to leave plan mode**, gets `behavior: "allow"`.

So Bypass, Default and Accept Edits behave alike. **Plan does not stop edits** (§7.3: the CLI *asks* before a write in
Plan mode, and "allow" lets it through) and it then approves its own plan; "Auto" depends on the CLI's classifier
(not observed) and then everything it asks about is allowed. `bypass` is spawned as
`--permission-mode default` on purpose: the bypass *flag* (`--dangerously-skip-permissions`) disables `can_use_tool`
routing altogether, which also kills `AskUserQuestion` (`buildRuntimeArgs.ts`, `CONTROL_PROTOCOL_FLAG`).

The menu text was corrected to say this (#4163), and corrected again after §7 showed #4163 had still called Plan
"read-only". That is a description of a limitation, not a fix.

### 2.2 What already exists

- `should_route_to_decision_panel(tool_name)`: the gate. Hard-coded `false`, with a long comment explaining it is a
  *policy* decision, not an engineering one: flipping it to `true` for everything makes every persistent agent a
  prompt-per-tool-call experience (`SPEC_AGENT_CONTROL_PROTOCOL` §7, "Permission chatter").
- `park_tool_permission_request` and `decide_tool_permission`: the parking and the answer path. Complete and tested.
- `AgentDecisionPanel.tsx`, `useAgentDecisions.ts`, the `tool:decision` IPC (`websocket.rs`): the UI and the wire.
  `SPEC_DECISION_PROMPT_2026_04_24.md` is "active — partially built" and offers four scopes.
- **Not built:** rule storage (that spec's §6) and any risk tiering (§7).

### 2.3 What is missing

1. **The gate does not know the mode.** srv reads `cmd:args`, not `agent:runtime`. The argv cannot tell Bypass from
   Default (both are `--permission-mode default`). The controller needs the *requested* mode at decision time.
2. **A policy** for what each mode routes.
3. **Allow rules**, so "prompt all" isn't unusable.
4. **A safe answer when nobody is there** (§2.5).

### 2.4 Proposal

The mode decides whether srv routes what the CLI asks about; the CLI's own mode still decides *what it asks about*.
The "CLI asks about" column is **observed** for Default, Accept Edits and Plan (§7.3) and **not observed** for Auto
(its classifier needs the real API).

| Mode | The CLI asks about | srv does |
|---|---|---|
| Bypass | (default mode: most non-read tools) | auto-allow, as today |
| Default | writes, reads outside the working directory, shell commands that are not known-safe (`touch`, `curl`; not `echo`) | route to the panel unless an allow rule matches |
| Accept Edits | the same minus edits (and minus `touch`-style file commands) | route to the panel unless an allow rule matches |
| Auto | what its classifier is unsure about | route to the panel unless an allow rule matches |
| Plan | **writes are ASKED about, not refused** (observed); `ExitPlanMode` asks | **srv must refuse writes itself** (deny `Write`/`Edit`/`NotebookEdit` and mutating shell commands with a message the model can act on): the CLI will not. Route `ExitPlanMode` as **"approve this plan?"**, which is the real plan approval |

Mechanics:

- The persistent controller reads `agent:runtime.permissionMode` from block meta when it spawns and keeps it in
  `PersistentInner` next to `spawn_runtime`. A mode change already restarts the process (`patchRuntime`), so the
  value cannot go stale.
- `should_route_to_decision_panel(mode, tool_name, input)` replaces the constant. `AskUserQuestion` stays separate.
- **Allow rules, minimum viable:** session-scoped, held in the controller, offered by the panel's existing scopes.
  Persisting them (that spec's §6) is a follow-up and is not needed for the first version to be usable.

### 2.5 The hard problem: prompts nobody can answer

A parked request blocks the turn until someone answers. Several callers have no human watching:
`agent.send` over the App API, jekt delivery, cron, the work queue, a pane closed mid-turn, a window that isn't
focused. Today none of them can hang on a prompt. After this change they could.

Required behaviour, in this order of preference:

1. If the pane is **visible and a panel is mounted**, park and wait.
2. Otherwise apply the pane's **unattended policy**: `allow` (today's behaviour; the default) or `deny` with a
   message the model can act on. Never hang.
3. A parked request older than a **timeout** (proposal: 10 minutes) resolves by the unattended policy and says so in the
   transcript.

The unattended policy is the owner's call (§2.7). The safe default is the one that preserves today's behaviour.

### 2.6 Testing

- Unit: the policy table (mode × tool × rule); `ExitPlanMode` routing; unattended fallback; timeout.
- The existing `tool_permission_tests` already cover park/decide; extend them with the mode input.
- Stub-process test (the harness in `persistent/tests/eager_resume.rs` already records what the process receives):
  a stub that emits a `can_use_tool` frame, assert it parks in Default and is auto-answered in Bypass.
- Frontend: the panel shows for Default and not for Bypass; plan approval wording.
- **Live, required before enabling:** one real session in each mode. The CLI's behaviour in `auto` and `plan` is
  described, not observed.

### 2.7 Decisions for the owner

1. Should **Default** really prompt for every tool the CLI asks about, or should the first version prompt only for
   the high-risk set (shell commands outside an allow list, writes outside the working directory) and keep the rest
   auto-allowed? The first is literally what the label said; the second is the usable one.
2. The **unattended policy** default: `allow` (status quo, safe for automation) or `deny`?
3. Ship **behind a setting, default off** until the live check passes? (Recommended.)

---

## 3. Item 2 — Remember the runtime across launches

### 3.1 What is true today

A fork copies its source pane's effective runtime (#4162). **Continue, Reattach and New session** start from a closed
agent with no live pane to read, so `agent:runtime` is seeded from the definition's flags and the catalog defaults.
Pick Opus / xhigh, close the pane, reopen the agent: Sonnet / high.

### 3.2 Where the state could live

`db_agent_instances` was dropped at schema v32. The agent's **latest launch state** now lives on the `db_agents` row
(v29: `session_id`, `status`, `started_at`, `ended_at`, plus `last_block_id`), written by the pane close / reopen
continuity write-back. One row per agent ⇒ one launch state per agent. A remembered runtime belongs there.

### 3.3 Proposal

- A `last_runtime` column on `db_agents` (JSON `{permissionMode, model, effort}`, empty = none), added as the next
  `OBJECT_SCHEMA_VERSION` migration (**currently 41; re-check the number when implementing**, other branches claim
  versions too).
- **Written only when the user chose something** — `patchRuntime` succeeded — not on every launch. Otherwise a pane
  that never touched the menu would "remember" the defaults and silently override the agent definition's own model
  (§3.7).
- Written at the existing close / continuity write-back point (it already reads block meta), so there is no new
  hot-path RPC. Also written when the setting is changed while the pane is open and the pane later crashes: the
  write-back on reopen covers it.
- Read by `launchAgentDefinition` through the existing carry-over parameter. New precedence for each setting:

  explicit launch choice **>** live fork source (#4162) **>** `last_runtime` **>** the definition's own flags **>** the
  provider default.

- Templates (`is_seeded`) store nothing: they are not agents anyone continues.

### 3.4 Edge cases

- **The provider changed** (an agent redefined from Claude to Codex): drop any stored value the new provider's
  catalog doesn't contain, per setting.
- **A stored model that left the catalog** (the overlay supersedes concrete ids): the dropup already migrates it;
  the same family-key rule applies on read.
- **Single-live-instance / Take over** (`SPEC_AGENT_SINGLE_LIVE_INSTANCE_2026_09_24`): the row is the shared record, so the
  runtime follows the agent across instances, which is the point.
- **Haiku and effort:** store what was chosen; the arg builder already omits `--effort` where it doesn't apply.

### 3.5 Testing

Migration (column added, old rows read as empty); write on close only after a user change; read precedence (the full
chain above); provider-change drop; template stores nothing; round-trip through a real reopen in the store test
harness; the frontend seeding function's table of precedence.

### 3.6 Not in scope

Per-**conversation** runtime (resuming an old session at the model it ran). The CLI records the model per message;
that is a different feature.

### 3.7 Decision for the owner

If an agent *definition* says `--model opus` and the user later picks Sonnet in the menu, should the next launch start
on **Sonnet** (the remembered pick) or **Opus** (the definition)? Recommended: Sonnet, because the user's explicit,
more recent choice wins and "reset to the definition" is a one-click menu action. The alternative makes the menu
feel like it forgets.

---

## 4. Item 3 — Show the model the CLI actually resolved

### 4.1 What is true today

The menu shows the selection, and (#4149) whether the **process was spawned with** that selection. The argv carries
an *alias* (`sonnet`); the CLI resolves it against the account and the provider. Anthropic's docs say an alias
resolves differently across providers and over time, and the repo already hit this once: the `sonnet` row read
"Sonnet 5.5" while CLI 2.1.280 resolved it to Sonnet 5. The menu cannot see that.

### 4.2 Sources, and how far each can be trusted

| Source | Gives | Status |
|---|---|---|
| `message_start.message.model` on the **main agent's** stream lines | the resolved model id of the last reply | Available now. Trustworthy since #4158 (subagent lines no longer pollute it). Lags: only known after a reply. |
| `system/init.model` | the resolved model at session start | **Observed (2.1.285):** emitted only after the first user message (~0.5 s later; nothing at all without one); carries the resolved `model` (`claude-sonnet-5-5` for `sonnet`). Too late to be the only source. |
| `get_settings` control request → `applied: {model, effort}` | the effective model **and effort**, on demand | **Observed (2.1.285): works immediately after spawn, with no account and no message.** `applied.model` is the resolved id; `applied.effort` is `null` for Haiku and `medium` for an unflagged Sonnet. srv sends only `interrupt` and `can_use_tool` answers today, so this is new protocol work. The only source for *effort*. |
| `modelUsage` on `result` | per-model cost | **Not a source.** It includes subagents' models (reconciliation report §3.6). |

### 4.3 Proposal, in two stages

**Stage A — frontend only, cheap, honest about its limits.**
Pane state already holds `lastContextModel` (now main-agent only). Show it where the menu already explains itself:
"Last reply used **Opus 5.5** (`claude-opus-5-5`)". Flag a mismatch **only when** all of these hold, because they
remove the timing false-positives:

1. the process was spawned with the selection (`compareRuntime` says it agrees), and
2. no restart is pending, and
3. the reported model's *family* differs from the requested one (`sonnet` vs an id containing `opus`), and
4. the reported id is recognised. Unknown shapes make **no claim**.

The comparison is by family substring for aliases and by prefix for concrete ids (a dated id extends an undated one).

**Stage B — srv readback.**
Send `get_settings` after spawn and after each `result`; fold `applied.{model,effort}` into the `agentruntime` event
as `effective`. The menu then shows effort too, which no stream source can. Old CLIs (no `get_settings`, or no
`applied`) fall back to Stage A and show effort as "requested".

### 4.4 Testing

Stage A: the family/prefix comparison table (aliases, concrete ids, dated ids, unknown shapes); the four gating
conditions each independently suppress the warning; the subagent case (a Haiku subagent's reply does not flag a Sonnet
pane). Stage B: against a **fake CLI** (§7) that answers `get_settings`, one that doesn't, and one that answers with a
different model than requested.

### 4.5 What was unknown, and is not any more

§7 answered the three things that blocked Stage B: the pinned CLI **does** answer `get_settings`; the response is
`{effective, sources, applied: {model, effort, advisor, ultracode, ...}}`; and it needs **no** authenticated session.
It also showed that `set_model`, `apply_flag_settings` (effort) and `set_permission_mode` take effect **on the running
process** with an `ok` answer, one request at a time (a request sent while another is outstanding can be answered out
of order), so Stage B can also *correct* drift without a restart (the reconciliation report's design).

---

## 5. Item 4 — Prune old CLI installs

### 5.1 What is true today

- `shared/cli/<provider>/<pin>/` — one directory per pinned version, ~200 MB each, finished installs carry
  `.agentmux-install-complete`. A pin bump adds one; **nothing removes any** (`cli_install::clear_unless_valid`
  only clears broken ones).
- `instances/v<AgentMux version>/cli/<provider>` — the pre-2026-09-27 per-version layout. 42 folders, 8.3 GB on the
  machine this was measured on. Read-only fallback for the *current* AgentMux version only.
- On that machine `shared/cli/claude` held 2.1.280 and 2.1.285.

### 5.2 Why this is a safety problem, not a cleanup

- **The shared dir is shared across AgentMux versions and channels, which can run at the same time.** A 0.58.2 build
  was running next to 0.59.1 on the machine this was written on. Its pin is the older one. Deleting "every pin but
  the newest" would pull the CLI out from under a live agent.
- **A restored pane stores the absolute CLI path** in `cmd` until its mount flow rewrites it. The old path must
  keep working briefly.
- **An install in progress** holds a cross-instance lock; deleting under it corrupts it.

### 5.3 Proposal: delete by *disuse*, never by version

1. **Record use.** Whenever `find_installed` / `ResolveCli` hands out an install, touch
   `<install dir>/.agentmux-last-used` (mtime only; throttled to once per hour per dir so it stays off the hot path).
   A spawn that uses a path records it the same way.
2. **A directory is prunable when ALL hold:**
   - it is **not the current pin** of any provider in this build's registry;
   - `.agentmux-last-used` (or, absent that, the install marker's mtime) is **older than 30 days**;
   - the install **lock can be taken without waiting** (`try_lock_install`);
   - **no live controller's CLI path is under it.** The controller registry knows the `cmd` each live pane was spawned
     with; this is the primary guard and works on every platform;
   - **no running process's executable is under it**, as a second guard against a CLI that outlived its controller. This
     cannot use `process_tracker`: it is a **no-op stub on every platform except Windows** (`process_tracker/mod.rs`),
     so it would silently approve everything on macOS and Linux. Use a direct scan (`/proc/*/exe` on Linux, `ps -axo
     comm` / `lsof +D` on macOS, the existing job-object tracker on Windows) and treat a failed scan as **"in use"**.
3. **Legacy folders** (`instances/v*/cli`): prunable when the version isn't the running one and the folder's newest
   mtime is older than 30 days. They are not read by any build after 2026-09-27 except as a same-version fallback.
4. **Never** follow symlinks out of the root, and apply the existing `is_safe_provider_component` /
   `is_safe_version_component` to every path before it is touched.
5. **When:** after a successful install, at most once a day, on a background thread at low priority, behind
   `AGENTMUX_NO_CLI_PRUNE=1`. It logs each removal with size and age.
6. **Visible:** a dry-run (`cli.prune` with `dry_run: true`) returning what *would* go and how much space, surfaced
   in the toolchain settings, so the first prune is never a surprise.

### 5.4 Why a 30-day age and not "keep N versions"

Version counting is wrong in both directions: a user who upgrades twice in a week would lose the CLI an older channel
still runs; a user who never upgrades would keep dead installs forever. Disuse is the quantity that matters.

### 5.5 Testing

On a temp `shared_dir` (the existing `paths_in` fixture): the current pin is never removed even if old; a recently
used old pin is kept; an old unused pin is removed; a held install lock blocks removal; a directory a live
controller's `cmd` points into is kept; a failed process scan keeps everything; a directory with a running process
path under it is kept; legacy folders for another version are removed only when old; symlinks and unsafe
components are skipped; dry-run changes nothing; the last-used touch is throttled; a crash mid-prune leaves nothing
half-deleted (remove the completion marker **first**, so a partly removed dir is already "not an install" and
`clear_unless_valid` finishes the job).

### 5.6 Unknowns

Whether mtime survives the platforms' backup / sync tools (a restored-from-backup tree resets mtimes to "now", which
only delays pruning, the safe direction); Windows file locking on a CLI that is still running (the in-use checks must
come first); how reliable the macOS process scan is under sandboxing. **Because the second guard can fail, the first
(the controller registry) and the age rule must each be sufficient on their own for the common case, and a scan that
errors must mean "keep".**

### 5.7 Not in scope

Pruning npm's own cache, or the per-channel data dirs (`channels/*/versions/*`), which hold stores, not binaries.

### 5.8 Decision for the owner

Is **30 days of disuse** the right line? Shorter reclaims space sooner and risks an occasionally-used older channel
re-downloading ~200 MB; longer is safer and slower to help. Also whether the first run should be **opt-in** (dry-run
visible, one click to prune) rather than automatic.

---

## 6. Smaller items

Each is small enough to be one PR. They are listed so they are not lost, in rough value order.

| ID | Item | Intended resolution | How to verify |
|---|---|---|---|
| G1 | `muxcode` / `openclaw` / `copilot` / `pi` get `--dangerously-skip-permissions` appended on every send; antigravity's `--yolo` is replaced by it | Give each provider an explicit permission vocabulary in the catalog (none / yolo / claude-style) instead of the Claude branch being the default; the ratchet test then lists none | Run each CLI with the flag against a stub (§7) and record whether it errors |
| G2 | Antigravity lists models but nothing applies one; `/model` says "applies to next turn" | Wire `--model` for it, or hide the picker *and* make `/model` and the stored default honour the same gate | Same stub run; the ratchet test (`KNOWN_MODELS_NOT_APPLIED`) shrinks |
| G11 | `/btw` copies the source pane's `cmd:args` | Run it through the same container-style heal (`container_argv`) so persistent-only flags (`--input-format stream-json`, `--permission-prompt-tool`) don't leak, and decide its model explicitly | A stub that records argv for a `/btw` on a persistent pane |
| G13 | The dropup silently restarts the agent when it migrates a superseded model id | Migrate the stored value without restarting (the next spawn picks it up), or ask | Dropup test: migration does not call the restart path |
| G14 | Only Claude panes have a runtime menu | A per-provider capability table (model? effort? mode?) drives both the menu and the slash commands; Codex gets a model-only menu | Table-driven test over the catalog |
| G15 | srv strips no `ANTHROPIC_MODEL` / `CLAUDE_CODE_EFFORT_LEVEL`; a `model` in `settings.json` applies where no flag is passed | Only matters where a flag is missing; now narrow. Document it; consider surfacing it in the "differs" explanation | Covered by the fake CLI (§7) |
| G16 | srv has no model list; `providers.models` is Claude-only and fetched with the account-global token | Per-provider models in the Rust registry, pinned to the frontend's by the existing consistency test; fetch with the agent's bound identity | Extend `runtime-defaults-consistency.test.ts` |
| G12 | Codex app-server reads `agent:model`, which nothing writes | Write it from `agent:runtime` when that controller is enabled | Only when enabled |

---

## 7. Verifying what we have only assumed

Several claims in the code and in this spec rest on how a third-party CLI behaves and have **never been observed**:

- the CLI takes the **last** of two repeated `--model` flags (everything here assumes it);
- which providers tolerate `--dangerously-skip-permissions` (G1);
- whether the pinned CLI answers `get_settings`, and with what (Item 3 Stage B);
- when `system/init` is emitted;
- how `auto` and `plan` modes route `can_use_tool` (Item 1).

**A way to find out without credentials, cost, or the macOS keychain prompt.** Point the CLI at a local fake API
server and an empty config dir:

- `CLAUDE_CONFIG_DIR=<empty temp dir>` and `ANTHROPIC_BASE_URL=http://127.0.0.1:<port>` with a dummy
  `ANTHROPIC_API_KEY`. The fake server returns a canned streaming response and records the `model` field of every
  request: that *is* the model the CLI resolved, with no real account involved.
- Spawn the real pinned binary (already installed under `shared/cli`) with the exact argv the pane builds, in
  stream-json mode, and send it the control requests under test.
- **Why this matters beyond convenience:** a test that uses the real login can trigger the keychain consent dialog
  (observed 2026-10-01 from an old build), and it spends the user's quota. The empty config dir plus a local base URL
  avoids both. Verify that claim first on a throwaway run; the CLI may still probe the keychain for an
  empty-config-dir login, in which case set the API key variable and confirm no prompt appears.

This harness is the single highest-value piece of infrastructure here: it turns five assumptions into tests, and it
is the "live check" the report keeps listing as outstanding, minus the need for a running AgentMux.

---

### 7.3 Results (pinned CLI 2.1.285, macOS arm64, 2026-10-01)

Run with `scripts/cli-probe` (a fake API, a `security` shim that answers "not found", a from-scratch environment).
**No account was used, no real keychain item was read, and no consent dialog appeared**; the shim's log shows the CLI
tried to read two items (`Claude Code-credentials-<hash>`, `Claude Code-<hash>`) and was told neither exists. Each
probe takes 3-6 seconds.

| Question | Observed |
|---|---|
| Which of two repeated `--model` flags wins? | **The last**, in both orders (`opus,haiku` → `claude-haiku-4-5-20251001`; `haiku,opus` → `claude-opus-5-5`). Everything that assumed this is confirmed. |
| What does an alias resolve to? | `sonnet` → `claude-sonnet-5-5`; `opus` → `claude-opus-5-5`; `haiku` → `claude-haiku-4-5-20251001` (the model id on the API request). |
| What runs with no `--model`? | **`claude-opus-5-5`, effort `medium`**: the 2026-09-29 incident, reproduced. |
| `--effort high` on Sonnet? | Sent as `output_config.effort: "high"`. |
| `--effort high` on Haiku? | **The CLI drops it**: no `output_config`, `get_settings` reports `effort: null`. So on this CLI the flag is **harmless** on Haiku. The earlier claim that it "400s" is true of older CLIs per `docs/providers/PROVIDER_MODELS_EFFORT_SETTINGS_2026-06.md`, and is **not** established for 2.1.285. Not passing it remains right; the menu's "not applied" is true. |
| When is `system/init` emitted? | Only after the first user message (~0.5 s later). Nothing at all before one. It carries `model` and `permissionMode`. |
| Does `get_settings` work? | Yes, immediately after spawn, with no message and no account. `applied` = `{model, effort, advisor, ultracode, ...}`. |
| Do control requests change a running process? | Yes: `set_model` (incl. `"default"` → Opus/medium), `apply_flag_settings` (`effortLevel`), `set_permission_mode` all answer `ok` and take effect; `get_settings` reflects the model and effort. Sent back-to-back, a later request can be answered before an earlier one finishes, so send one at a time. |
| Permission routing, **with** a permission prompt tool (a persistent agent) | `default`: asks for Write, `touch`, `curl`, and a read outside the working directory; does **not** ask for `echo`. `acceptEdits`: no ask for Write or `touch`; asks for `curl` and the outside read. **`plan`: asks for Write (does not refuse it)**, asks for `ExitPlanMode`; `echo` and an in-tree read are not asked. A **deny** answer produces a real tool error in every case. |
| Permission routing, **without** one (a container's one-shot run) | `default` refuses an unapproved write and `touch`; `plan` refuses writes ("Cannot write to …") and blocks commands. There, "Plan (read-only)" is true. |

**What this overturned.** #4163 said Plan on a persistent agent is "read-only while planning". It is not: the CLI asks
before writing and the server's blanket "allow" lets the write happen. The wording was corrected (and the earlier
"Default (prompt all)" was wrong in a second way: even the CLI's own Default does not ask about safe commands, and a
one-shot run *refuses* instead of asking).

**Not observed, and why.** Auto mode (its classifier is a model call the fake API cannot emulate); Windows; the other
providers' CLIs (not installed here), so G1 stays unverified; the CLI's behaviour under the real API's validation.

**Reproduce:** `AGENTMUX_CLI_PROBE=1 npx vitest run scripts/cli-probe/cli-probe.test.mjs` (opt-in; it spawns the pinned
CLI from `~/.agentmux/shared/cli`).

---

## 8. Order, effort, risk

| Order | Item | Size | Risk | Needs |
|---|---|---|---|---|
| 1 | §7 harness (fake API server + argv runner) | small–medium | low | **built; results in §7.3** |
| 2 | Item 4 install pruning | medium | **medium** (deletes files) | §5.8 decision; ships with dry-run first |
| 3 | Item 3 Stage A | small | low | nothing |
| 4 | Item 2 remembered runtime | medium | low–medium (schema) | §3.7 decision |
| 5 | Item 3 Stage B | medium | low | nothing: §7 showed `get_settings` works |
| 6 | smaller items (§6) | small each | low | §7 for G1 / G2 |
| 7 | Item 1 approval prompts | **large** | **high** (can hang unattended agents) | §2.7 decisions; behind a setting |

Reasoning: the harness is cheap and removes uncertainty from everything after it; pruning is the clearest user-visible
cost; approval prompts last because they are the only item that can make an existing workflow worse.

## 9. What this spec does not claim

- It does not claim any of the proposals are safe to ship without the tests listed; it lists them so they exist.
- It does not decide the owner's questions (§2.7, §3.7, §5.8); it recommends and says why.
- Nothing here has been run in a real build. The harness in §7 is how that changes.
