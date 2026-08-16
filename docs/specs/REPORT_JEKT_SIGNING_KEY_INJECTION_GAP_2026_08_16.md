# Jekt Signing-Key Injection Gap — Normal-Launch Agents Never Got a Verified Identity

**Date:** 2026-08-16
**Author:** Clamk (agent, `~/.agentmux/agents/clamk-0612a`)
**Status:** Root cause confirmed with direct evidence (live `.mcp.json`, server logs, code); fix implemented,
tested, and included in this same change (see §4). Not yet merged.
**Ground truth basis:** `agentmuxai/agentmux` `origin/main` at `72aefad4d` / worktree branch
`clamk/agent-identity-history-protocol`. Live evidence gathered directly from this machine's own running
AgentMux instance (srv `v0.55.9`) — not inferred from code reading alone.

---

## 0. How this was found

Unrelated to this report's subject: this agent sent an outgoing jekt (agent-to-agent message) and received
one back that rendered `TRUST=self-declared` rather than `TRUST=host-verified`, despite both agents running
on the same machine (`DELIVERY=host`) — the case the host-tier signing feature
(`SPEC_JEKT_TRUST_LAYER_COMPLETION_2026_08_13.md`) exists specifically to cover. Investigating why turned up a
real, currently-live gap, not a stale/outdated build.

---

## 1. Direct evidence, in order

### 1.1 This agent's actual `.mcp.json` has no signing keys

```
$ cat C:/Users/asafe/.agentmux/agents/clamk-0612a/.mcp.json
{
  "mcpServers": {
    "agentmux": {
      "type": "stdio",
      "command": "agentmux-mcp",
      "args": [],
      "env": {
        "AGENTMUX_AGENT_ID": "clamk"
      }
    }
  }
}
```

No `AGENTMUX_JEKT_KEY`, no `AGENTMUX_LAN_KEY` — only `AGENTMUX_AGENT_ID`.

### 1.2 That file is current, not stale

```
$ stat C:/Users/asafe/.agentmux/agents/clamk-0612a/.mcp.json
Modify: 2026-08-16 05:20:53.262266900 -0700   (= 2026-08-16T12:20:53Z)
```

### 1.3 The running server version already had both signing features for three+ days

`VERSION_HISTORY.md`:
```
## 0.55.9 — 2026-08-15
- feat(jekt): per-agent Ed25519 signing for LAN-tier jekts
## 0.55.7 — 2026-08-13
- feat(jekt): add host-tier per-agent HMAC signing for sender verification
```

`~/.agentmux/logs/current-srv-v0.55.9.path` was written 2026-08-16 05:17 (local), three minutes before the
`.mcp.json` write in §1.2, and the server's own startup log line confirms it was actually running that
version at that time:

```
{"timestamp":"2026-08-16T00:24:54.946388Z", "message":"agentmuxsrv starting","version":"0.55.9", ...}
```

So the `.mcp.json` that has no keys was written by a server build that had shipped both signing features 3
and 1 days earlier, respectively. This rules out "hasn't been rebuilt since the feature shipped" as the
explanation.

### 1.4 The write event is identified precisely in the server log, and it isn't `agent_open`

`agentmuxsrv-v0.55.9.log.2026-08-16` (JSON lines, UTC timestamps) has, at the exact second `.mcp.json` was
last modified:

```json
{"timestamp":"2026-08-16T12:20:53.256237Z","level":"INFO","fields":{"message":"WriteAgentConfig","working_dir":"C:\\Users\\asafe\\.agentmux\\agents\\clamk-0612a","file_count":9,"auto_allocate":false},"target":"agentmux_srv::server::editor_handlers"}
```

Searching the entire log file for the strings that would appear if `agent.open`'s Rust-side key injection had
run: zero matches for `agent_open`, zero matches for `jekt` (case-insensitive), across the whole file. The
event that actually wrote this file was `WriteAgentConfig` (`agentmux_srv::server::editor_handlers`), not
`agent.open`.

### 1.5 Why: `WriteAgentConfig` is the real "click Launch" path, and never called the injection code

Two independent code-path facts, both confirmed by reading the source (not inferred):

- **`agent_open.rs` DID contain the injection logic** (before this fix — see §4): `wstore.agent_jekt_key_ensure(agent_slug)` / `wstore.agent_lan_key_ensure(agent_slug)`, called from `agent.open`'s `write_agent_config_files`, patching `.mcp.json`'s `mcpServers.agentmux.env` before writing.
- **`editor_handlers.rs`'s `WriteAgentConfig` handler is a separate RPC path** that just persists whatever `cmd.files` the caller supplies — its own doc comment (line 253-258, pre-fix) already says this is *"the actual 'click Launch' path used on every normal agent launch (`launchAgentDefinition`/`WriteAgentConfigCommand`)"*, shared with `agent.open` only for skill-file cleanup logic (`agent_config.rs`), **not** for jekt-key injection.
- The frontend function that actually builds `.mcp.json` content for this path, `frontend/app/view/agent/agent-config-builder.ts`, sets exactly one env var: `agentMuxServer.env["AGENTMUX_AGENT_ID"] = slug || instanceName || agent.name;` (line 432) — no jekt/LAN key handling exists there at all, confirmed by a full-file grep for `JEKT_KEY`/`LAN_KEY`/`jekt` (zero matches).

**Conclusion:** the host-tier and LAN-tier jekt signing-key injection, shipped 2026-08-13 and 2026-08-15, was
only ever wired into the `agent.open` RPC path. The path an agent actually launches through on a normal
"click Launch" — `WriteAgentConfig`, driven by the frontend's `agent-config-builder.ts` — never called it.
Since this machine's evidence (§1.1-1.4) shows this agent went through `WriteAgentConfig`, not `agent.open`,
its jekts have been rendering `TRUST=self-declared` this whole time despite the signing feature being live in
the running binary. This is not specific to this one agent — any agent launched the normal way is affected
the same way, on any install running v0.55.7+.

---

## 2. Why this matters

Per `CLAUDE.md`'s jekt security rules, `TRUST=self-declared` and `TRUST=host-verified` are treated very
differently: a self-declared sender's identity is *not proven*, which is precisely the gap the signing
feature was built to close (`SPEC_JEKT_TRUST_LAYER_COMPLETION_2026_08_13.md`'s own incident writeup: a
spoofed jekt asking for a GitHub PAT, followed by a spoofed "confirmation" over muxbus). If the primary launch
path never actually equips an agent with a signing key, that protection is live in the codebase but not in
practice for ordinary agents — a real security-relevant gap between shipped code and shipped behavior, not
just a cosmetic trust-badge issue.

---

## 3. What this is *not*

- **Not a version-lag issue.** §1.3 rules this out directly — the running binary is newer than both features.
- **Not specific to this agent.** The root cause is in shared frontend/backend code (`agent-config-builder.ts`, `editor_handlers.rs`) used by every agent's normal launch, not anything particular to `clamk-0612a`'s config.
- **Not a signing-implementation bug.** `agent_jekt_key_ensure`/`agent_lan_key_ensure` and the HMAC/Ed25519 mechanics themselves are untouched by this report — they simply were never invoked for this code path.

---

## 4. Fix implemented (this change)

Rather than duplicating `agent_open.rs`'s injection logic into `editor_handlers.rs` (which would reintroduce
exactly the kind of drift `agent_config.rs`'s own module doc already warns about for the skill-file-cleanup
logic it centralizes), extracted the injection into one shared, pure, unit-tested function:

- **`agent_config::inject_jekt_signing_keys_into_mcp_json(content, wstore, agent_slug) -> Option<String>`**
  (`agentmux-srv/src/backend/agent_config.rs`) — mints-or-reuses both keys, patches
  `mcpServers.agentmux.env`, returns `None` (leave content untouched) on any parse failure or if there's no
  `env` object to patch. Best-effort, matching the existing design philosophy: never blocks a spawn.
- **`agent_open.rs`** now calls this shared function instead of its former ~75-line inline duplicate (both
  the host-tier and LAN-tier blocks collapsed into one call + one warn-on-`None`).
- **`editor_handlers.rs`'s `WriteAgentConfig` handler** now also calls it — extracting `agent_slug` from the
  `.mcp.json` file already present in `cmd.files` (via `AGENTMUX_AGENT_ID` in its `env` block) before writing.
  This is the actual fix: the normal "click Launch" path now injects keys too.

**Tests added** (all passing, `cargo test -p agentmux-srv` — 2351 passed, 0 failed, 6 pre-existing ignores):
- `agent_config::tests::inject_jekt_signing_keys_into_mcp_json_patches_both_keys_into_the_env_block`
- `agent_config::tests::inject_jekt_signing_keys_into_mcp_json_reuses_the_same_key_on_a_second_call`
- `agent_config::tests::inject_jekt_signing_keys_into_mcp_json_returns_none_when_theres_no_env_object_to_patch`
- `agent_config::tests::inject_jekt_signing_keys_into_mcp_json_returns_none_on_malformed_json`
- `editor_handlers::tests::writeagentconfig_extracts_slug_and_injects_signing_keys_into_the_real_mcp_json_shape`
  — exercises the exact slug-extraction-then-injection sequence the real handler runs, against the real
  `rpc_types::AgentConfigFile{path, content}` shape (not `agent_config::AgentConfigFile{filename, content}` —
  the two are distinct types with different field names; this repo has been bitten by that mismatch silently
  no-op'ing a shared code path before, per the neighboring `writeagentconfig_files_produce_the_expected_managed_skill_paths`
  test's own comment).

**Not fixed by this change:** any agent whose `.mcp.json` was already written before this fix lands still has
no key — it will get one on its *next* successful `WriteAgentConfig` or `agent.open` call (i.e., its next
launch), matching the existing best-effort/self-healing design already documented in both call sites'
comments. No backfill migration was written for already-materialized `.mcp.json` files; relaunching is
sufficient and matches how the feature already described its own degrade path ("until the next successful
spawn").

---

## Appendix: research method

Every claim above traces to a direct artifact on this machine or a direct source read, not inference:
`.mcp.json` content and mtime were read directly; the server log line was matched by exact UTC timestamp
against the file mtime (converted from the local `-0700` stat output); the version/feature-ship dates came
from `VERSION_HISTORY.md`; the "which code path ran" conclusion came from a full-file string search for
`agent_open`/`jekt` in the actual log file (absence, not presence, is the load-bearing evidence there — cross-
checked by confirming `clamk` DOES appear 22 times in the same log, ruling out "wrong log file"). The
frontend claim (`agent-config-builder.ts` sets only `AGENTMUX_AGENT_ID`) was a direct grep of that file, not
a recollection.
