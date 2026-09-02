# REPORT — cross-agent message delivery silently fails for every subprocess-controller agent

**Date:** 2026-09-02
**Author:** Agent5
**Status:** Diagnosed on live v0.55.31 logs + `main` @ `01cd708e6`; fix in this PR
**Severity:** P1 — every jekt/muxbus message to an affected agent is dropped, and
the sender is told it failed while the swarm pane still lists the agent as present.

---

## 1. Symptom as reported

An operator ran a container agent (`Scouto`) alongside a host agent (`Agent5`) on
the same machine. From the operator's side:

> "the swarm entry says he is here, but no response in the actual pane"

Two separate problems were tangled together here. Section 2 is the one that was
already self-healing (credentials); section 3 is the real, still-live defect.

---

## 2. Not the bug — the credential gate (already resolved by re-login)

`Scouto`'s first failure was a genuine, correctly-reported credential problem:

```
14:40:26 WARN identity: identity.spawn.blocked: no credentials for provider claude
         (definition fb770209-2029-4100-b01e-16fa89904cac) — account
         3dfe5ef7-7255-4de1-aae4-4df6856f4aee row not found; spawn refused
         (single-point enforcement — use_ambient_login=false, ignored)
```

This is `SpawnGateError::MissingCredentials`
(`agentmux-srv/src/identity/resolver/errors.rs`) behaving exactly as
`SPEC_ACCOUNT_DELETE_DEAUTH_LAYERS_2_4_2026_07_14.md` §2.2 specifies: an
oauth-class provider whose bound account row is gone fails the spawn closed
rather than silently falling through to the operator's global `~/.claude` login.

The operator's "Login Again" fixed it, and the log shows the full recovery:

```
14:40:29 INFO identity: auth.start (direct-account): OAuth config dir wired
         account_id="3dfe5ef7-…" env_var="CLAUDE_CONFIG_DIR"
14:40:36 INFO auth.credstate: present=true token=e564450e refresh=true
14:40:38 INFO identity: injected CLAUDE_CONFIG_DIR for oauth provider claude
```

**Credentials are not the open issue.** The agent still did not respond, which is
what led to the actual defect below.

---

## 3. The bug — `deliver_agent_message` has no path for subprocess controllers

### 3.1 Observed

A jekt sent from `Agent5` to `Scouto`, in the same log, seconds later:

```
14:42:58 INFO  reactive inject request received target_agent=Scouto source_agent=Some("agent5")
14:42:58 INFO  inject: sending payload to PTY target_agent=Scouto msg_len=2096
14:42:58 ERROR inject: sender failed target_agent=Scouto
               error=subprocess controller does not accept raw input; use AgentInputCommand
```

Note the middle line: delivery fell back to **PTY keystroke injection**. The
target has no PTY.

### 3.2 Mechanism

`backend/blockcontroller/mod.rs::deliver_agent_message` is the single
controller-aware delivery primitive behind muxbus Tier-1. It handles exactly two
controller kinds and defaults everything else to "use keystrokes":

```rust
pub fn deliver_agent_message(block_id: &str, message: &str) -> Result<AgentDelivery, String> {
    let ctrl = get_controller(block_id)...;
    if let Some(persistent_ctrl) = ctrl.as_any().downcast_ref::<persistent::PersistentSubprocessController>() {
        persistent_ctrl.send_user_message(message.to_string())?;
        return Ok(AgentDelivery::Structured);
    }
    if ctrl.controller_type() == BLOCK_CONTROLLER_ACP {
        ctrl.send_input(BlockInputUnion::data(message.as_bytes().to_vec()), None)?;
        return Ok(AgentDelivery::Structured);
    }
    Ok(AgentDelivery::Pty)   // <-- SubprocessController lands here
}
```

`SubprocessController` then refuses the keystrokes it is handed, by design
(`backend/blockcontroller/subprocess/mod.rs:401`):

```rust
if input.input_data.is_some() {
    return Err("subprocess controller does not accept raw input; use AgentInputCommand".to_string());
}
```

Both halves are individually correct. The gap is that nothing bridges them: a
`SubprocessController` starts a turn only via `AgentInputCommand`, and
`deliver_agent_message` never issues one. The `AgentDelivery::Pty` branch is
documented as covering "shell/term PTY agents, one-shot subprocess agents" —
lumping a controller that *rejects* PTY input in with controllers that *want* it.

### 3.3 Blast radius — much wider than container agents

`SubprocessController` is not a container-only concern. From
`backend/providers.rs`:

| Provider | Controller | Affected on host? |
|---|---|---|
| claude | Persistent | no |
| codex | **Subprocess** | **yes** |
| gemini | **Subprocess** | **yes** |
| qwen | **Subprocess** | **yes** |
| kimi | **Subprocess** | **yes** |
| muxcode | **Subprocess** | **yes** |
| antigravity | **Subprocess** | **yes** |
| openclaw / pi / copilot | Acp | no |

Additionally **every container agent of any provider** is affected, because
`app_api/agent_open.rs:297` deliberately forces `controller_type = "subprocess"`
for `agent_type == "container"` (a container turn is one `docker exec`; a
long-lived persistent stdin cannot express that). So a container *Claude* agent —
the one provider that is safe on the host — becomes affected the moment it is
containerised. That is precisely the `Scouto` case.

Net: 6 of 10 providers as host agents, plus 10 of 10 as container agents, cannot
receive agent-to-agent messages at all.

### 3.4 Why the swarm pane still shows the agent as healthy

Registration and delivery are separate code paths. `register_agent` /
`reactive::registry::write` run on the *spawn* path and succeed normally — which
is why `DiscoverAgents` lists `Scouto` as `addressable: true`, and why the
operator saw "the swarm entry says he is here". Nothing in the registry reflects
that the delivery primitive has no route to this controller type. The result is a
confidently-wrong presence indicator.

---

## 4. Fix implemented in this PR

**Bridge the two halves: when the target is a `SubprocessController`, start a
turn the same way `AgentInputCommand` does, instead of falling back to PTY.**

The full spawn-config resolution an agent turn needs (block meta re-read, argv
healing, identity injection + spawn gate, muxbus/bashwrap env, container
`ensure_running` + `spawn_container_turn`, agent re-registration) already exists —
twice — in `server/agent_handlers/input.rs` (the `agentinput` RPC handler) and
`server/app_api/agent_io.rs` (the `agent.send` App API handler). It needs
`AppState` and it is `async`; `deliver_agent_message` has neither.

Changes:

1. **`server/agent_handlers/input.rs`** — extract the `agentinput` handler body
   verbatim into `pub async fn run_agent_turn(deps, block_id, message,
   message_id)`, plus an `AgentTurnDeps` struct holding the nine `AppState`
   pieces it needs. The RPC handler becomes a thin caller. No behavior change to
   the existing path; the helpers (`container_argv` et al.) and their tests are
   untouched.

2. **`bootstrap.rs`** — `install_agent_turn_delivery(state)` re-installs the
   reactive handler's `MessageSender` with a version that closes over
   `AgentTurnDeps`. It keeps today's behavior for persistent/ACP controllers
   (delegating to `deliver_agent_message`) and adds the missing branch: for a
   `SubprocessController`, run `run_agent_turn` and report `Structured` rather
   than falling through to keystrokes.

3. **`main.rs`** — call it immediately after `build_app_state`, matching the
   existing post-state wiring calls (`native_memory_drift::spawn`, etc.). The
   pre-`AppState` sender installed in `spawn_background_subsystems` stays as the
   early-boot fallback.

### 4.1 Honest reporting of an async delivery

`MessageSender` is synchronous (`Fn(&str, &str) -> Result<bool, String>`) while
`run_agent_turn` is async, so the turn is spawned onto the Tokio runtime and the
sender returns before it completes. To keep the success signal meaningful, the
closure performs the checks it *can* do synchronously — controller present,
controller is a `SubprocessController`, Tokio handle available — and only then
reports acceptance. Failures after that point (identity gate refusal, Docker
unavailable, `ensure_running` failure) surface in the agent pane through the same
persisted `error_during_execution` frame and `agent:last_failure` recovery card
that a UI-initiated turn uses. This matches the semantics the PTY path already
had: writing keystrokes never guaranteed the agent acted on them either.

The distinction that matters, and which this PR fixes, is between *"delivered to
a controller that can act on it"* and *"handed to a controller that rejects it
outright"*. Only the latter was happening before.

---

## 5. What this PR does not change

- The credential spawn gate (§2) — working as specified.
- `deliver_agent_message` itself — left as the persistent/ACP primitive it is,
  rather than given an `AppState` dependency it has no business holding
  (`backend::` must not depend on `server::`).
- The duplicate spawn-config logic in `app_api/agent_io.rs`. It is now a third
  copy of a body that exists twice; folding it into `run_agent_turn` is a
  worthwhile follow-up but is a wider blast radius than this fix needs, and
  `agent.send` additionally returns a captured `session_id` that the other two
  callers do not.
- Container-agent UX gaps from `SPEC_HOST_VS_CONTAINER_AGENTS_2026_06_18.md`
  (runtime badges, container-by-default, host warning) — unrelated, still open.

---

## 6. References

- `agentmux-srv/src/backend/blockcontroller/mod.rs:374-406` — `deliver_agent_message`
- `agentmux-srv/src/backend/blockcontroller/subprocess/mod.rs:400-402` — the refusal
- `agentmux-srv/src/backend/reactive/handler.rs:733-746` — the PTY fallback + error log
- `agentmux-srv/src/bootstrap.rs:997-1003` — pre-`AppState` `set_message_sender`
- `agentmux-srv/src/server/app_api/agent_open.rs:294-301` — container forces subprocess
- `agentmux-srv/src/backend/providers.rs` — per-provider controller types
- `docs/specs/SPEC_CONTAINER_PANE_SUPPORT_2026_06_11.md` — container turn design
- `docs/specs/SPEC_AGENT_CONTROL_PROTOCOL_2026_06_15.md` §6 — the Phase-3 delivery
  primitive this extends
- `docs/specs/SPEC_ACCOUNT_DELETE_DEAUTH_LAYERS_2_4_2026_07_14.md` §2.2 — §2's gate
