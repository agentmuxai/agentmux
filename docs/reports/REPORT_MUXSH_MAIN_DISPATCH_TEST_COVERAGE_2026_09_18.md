# Scope: `muxsh` main()/apiCall() test coverage — 2026-09-18

**Status:** proposed → implementing this session
**Why:** `muxsh.test.mjs`/`muxsh.contract.test.mjs` thoroughly cover `parseArgs`,
`buildRequestBody`/`buildShellCreateBody`/`buildAgentSendBody`, and every
`render*` function (477 + 131 lines of pure-logic tests against 541 lines of
source). Nothing exercises `main()` or `apiCall()` — the dispatch switch,
HTTP-error handling, and `fail()`/exit-code paths in
`agentmux-srv/src/backend/shellintegration/muxsh.mjs:407-537`. Surfaced while
investigating a user-reported "basic stuff" error (which turned out to be the
already-documented tool-spawned-shell gap, `docs/MUXSH.md:18-19` — unrelated
to this gap, but prompted auditing coverage while in the file).

## In scope

New file, `agentmux-srv/src/backend/shellintegration/muxsh.main.test.mjs` —
kept separate from `muxsh.test.mjs` because that file's own header states
"pure, no network, no running instance"; these tests mock `fetch` and
`process.exit`, which would contradict that contract if added there.

Mocking approach: `vi.stubGlobal("fetch", ...)` (same pattern already proven
in `lib/muxclient.test.mjs`) plus `vi.spyOn(process, "exit").mockImplementation(() => { throw new ProcessExitError(); })`
so a `fail()` call can be asserted on without actually killing the test
process, and `console.log`/`console.error` spied to assert rendered output.

Cases:
1. **`apiCall` 404 with a `notFoundHint`** — asserts the "this AgentMux
   instance may predate ..." hint is appended (currently untested;
   `muxsh.mjs:421-423`).
2. **`apiCall` non-JSON error body** — asserts fallback to `HTTP <status>`
   when `result.resp.json()` throws (`muxsh.mjs:414-419`).
3. **`apiCall` network failure** (`agentmuxFetch` rejects) — asserts exit
   code 2 and the "cannot reach ..." message (`muxsh.mjs:408-413`).
4. **Missing env** — `AGENTMUX_LOCAL_URL`/`AGENTMUX_AUTH_KEY` unset → exit 1
   with the "not set" message, for every command that isn't `config-path`
   (`muxsh.mjs:446-452`).
5. **`agent-send` with `success: false`** — asserts exit code 2 even though
   the HTTP response itself is 200 OK (`muxsh.mjs:519-533`; this is the one
   path with an explicit code comment flagging it as important but it was
   never actually tested).
6. **Each dispatch branch reaches the right route** — one happy-path test
   per `parsed.command` (`open`, `web`, `config-edit`, `pane-list`,
   `run-create`, `run-status`, `run-stop`, `agent-list`, `agent-send`)
   asserting the URL/method passed to the mocked `fetch`, so a future typo
   in the dispatch switch's route string fails a test instead of shipping.
7. **`config-path`/`config-edit` env-var gates** — `AGENTMUX_CONFIG_DIR`
   unset → exit 1 for `config-edit`; each of `data`/`config`/`logs`/`shared`
   reads its own env var for `config-path` (`muxsh.mjs:438-444,464-467`).

## Out of scope (per prior discussion — not revisited here)

- Shell-delegation-layer tests (`bash.sh`'s `muxsh()` function, and its
  zsh/fish/pwsh equivalents) — would need a real spawned-shell harness,
  low value since the one gap in that layer is already documented behavior,
  not an uncaught bug.
- Full e2e against a real running srv instance — no evidence of an actual
  bug there today; the contract test already guards the highest-value
  failure mode (client/server field drift).

## Done means

`npm test` (vitest) passes with the new file included, every case above
exists and fails if the corresponding `muxsh.mjs` line it guards is reverted
(spot-checked by temporarily breaking one line per case during writing, not
just written and assumed correct).
