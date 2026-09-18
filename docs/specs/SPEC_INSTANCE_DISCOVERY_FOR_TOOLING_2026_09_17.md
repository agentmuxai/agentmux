# SPEC: tooling must discover an instance, not assume a well-known port

**Author:** Opaz
**Date:** 2026-09-17
**Status:** implemented

---

## 1. Problem

Agent and test tooling hardcodes the CEF debug port — "9223 dev / 9222 release":
`tools/tests/bench-agent-keystroke.mjs`, `tools/tests/agent-layout-drift.mjs`,
`tools/tests/bench-aggregate.mjs`'s documented invocation, and `docs/linux.md`.

Those are only **preferred** values. `agentmux-cef/src/lib.rs` binds the
preferred port if it is free and otherwise takes an OS-assigned one — deliberately,
because running several instances in parallel is a designed feature (I1–I6), and a
fixed port collides. The behaviour is correct; what is missing is any way for a
caller to learn which port it got.

So on a machine with more than one instance up — the normal case for this project
— the constant identifies **whichever instance won the race**, not the one the
caller meant.

### 1.1 The failure is silent, and it happened

While debugging a packaged build, an agent attached to port 9222 believing it was
its own test instance. 9222 belonged to the repo owner's live instance, which had
started first. The agent dispatched mouse clicks and synthetic keystrokes into it.
Nothing errored. The mistake was caught afterwards, by noticing the debugger's page
targets were served from a different port than the instance under test.

A tool that fails loudly wastes a minute. A tool that silently targets the wrong
instance corrupts someone else's session and produces test results that look
valid.

## 2. Why the port was undiscoverable

The value is not secret. It is logged at startup:

```
CEF remote-debugging port: 42149 (preferred 9222)
```

and stored in `app_state.debug_port` for `browser_api`. It simply was not
published anywhere a tool reads.

`authkey.dev` already exists **for exactly this purpose** — `SPEC_TEST_API_ACCESS.md`
and `SPEC_BENCHMARK_PORTABLE_DISCOVERY_2026_05_20.md` describe it as how harnesses
discover a running instance without manual flags — and it publishes the web, ws and
ipc endpoints. It did not publish this one.

## 3. Design

### 3.1 Publish it

`debug_port` is added to `authkey.dev`. It is resolved before the file is written
(`lib.rs`), which is why the port-selection block now sits above the authkey block
rather than beside the CEF settings.

Release builds write the file too, so this is not dev-only.

### 3.2 Discover, verify, and refuse

`tools/tests/lib/instance-discovery.mjs`:

- `listLiveInstances()` — every `authkey.dev` under `~/.agentmux/{channels,dev}`
  whose `host_pid` is still alive, newest first.
- `resolveInstance({ match })` — exactly one instance. With several live and no
  `match`, it **throws and lists the candidates** rather than picking. Guessing
  here is the defect, so guessing is not an option the API offers.
- `resolveCdpPort(inst)` — returns the port only after confirming the debugger's
  page targets are served by that instance's own web endpoint. The port number
  alone does not prove whose it is.
- An instance predating the field reports `debugPort: null` and
  `resolveCdpPort` throws with instructions, rather than falling back to 9222.
  **The fallback is the bug.**

`root` and `alive` are injectable so the module is testable without a running
instance.

## 4. Tests

`tools/tests/lib/instance-discovery.test.mjs` (node:test, per the
`tools/tests/lib/` convention):

- a dead instance's authkey is ignored
- the port comes from the file, not a constant (asserts 42149, not 9222)
- an instance predating the field reports `null` rather than defaulting
- **ambiguity throws** — the case that caused the incident
- `match` selects; a match that hits nothing errors and lists what is live
- no live instances is an error, not an empty pick

`agentmux-cef`: `the_authfile_publishes_the_debug_port` asserts the field is
serialized with the actual bound port.

## 5. Generalisation

This is the fourth instance of one pattern found in a single session. Each is a
**machine-global constant used as an identity**:

| Resource | Constant | Consequence |
|---|---|---|
| Shell integration scripts | `~/.agentmux/shell/` | versions overwrote each other's rcfiles |
| AppImage extraction cache | `extracted/$VERSION` | a new build silently ran an older binary |
| Instance identity in env | inherited `AGENTMUX_*` | a build adopted the launching instance's channel |
| CDP debug port | `9222` / `9223` | a tool drove a different instance than intended |

The rule: **a well-known constant cannot identify an instance.** When N
instances are supported by design, any fixed name — path, port, env var —
resolves to "whoever got there first". Identity has to be discovered from
per-instance state and verified, and ambiguity has to be an error.

## 6. Out of scope

- Other `tools/tests/*.ps1` harnesses still discover via `authkey.dev` by their
  own path logic; they were not touched.
- `agentmux_common::make_cli_cmd` and the MCP server have their own discovery
  paths and are unaffected.
