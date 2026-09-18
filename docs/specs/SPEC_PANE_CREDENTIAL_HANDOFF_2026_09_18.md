# SPEC: panes should not hold the instance's API credential

**Author:** Opaz
**Date:** 2026-09-18
**Status:** proposed

---

## 1. Problem

`AGENTMUX_AUTH_KEY` is injected into every spawned agent's environment
(`server/agent_handlers/input.rs`) and is the **sole** authentication factor for
the entire App API. `auth_middleware` (`server/mod.rs`) is a string comparison:

```rust
match auth_key {
    Some(key) if key == state.auth_key => next.run(req).await,
    _ => (StatusCode::UNAUTHORIZED, ...)
}
```

No scope, no expiry, no per-caller identity, no rate limit. One value opens
every `/api/v1/*` and `/agentmux/*` route behind that middleware.

Because it lives in the environment, **every descendant process inherits it**:
the agent CLI, its shell tool, anything that shell runs — an `npm install`
postinstall script, a downloaded binary, a language server. On Linux it is also
readable by any same-uid process via `/proc/PID/environ`.

This is a textbook confused-deputy setup: authority travels *ambiently* with the
process tree rather than with the request, so a process that was never meant to
act on the instance can, and the server cannot tell the difference.

### 1.1 Blast radius

A holder can do anything the UI can, because the frontend authenticates the same
way: create and kill panes and shells, open editors, read and write block files,
drive the UI, send messages as any agent, manage cron, read native and global
memory, fleet-broadcast.

`/ws` additionally accepts the key as a `?authkey=` query parameter (the browser
WS API cannot set headers). The existing comment there already notes this is
preserved in history, `Referer`, and access logs — a CSRF amplifier once the key
leaks.

### 1.2 This is already understood for one surface

`service/credential.rs` documents the exact attack:

> an agent reads `AGENTMUX_AUTH_KEY` and `AGENTMUX_BLOCKID` from its own
> environment (both injected at spawn) and curls `credential.Fill` to get a
> stored plaintext password with no human approval anywhere in the path

and closes it with a **second factor** — `AGENTMUX_HOST_REG_SECRET`, which is
host-only and never reaches a pane. It refuses to fall back:

> Refusing rather than falling back to the shared X-AuthKey, which every agent
> can read from its own environment.

That is the right instinct, applied to one method. Everything else behind
`auth_middleware` still trusts the ambient key alone.

## 2. What is already fixed, and what this does not re-litigate

- Shell panes never receive the key: `config.rs` removes it from srv's own
  environment at startup, so PTY inheritance cannot carry it.
- Agent panes receive it deliberately, by explicit re-injection, because
  bashwrap needs `/agentmux/wps/publish`.
- `SPEC_PANE_ENV_ISOLATION_2026_09_17` stopped panes inheriting instance
  *identity* (channel, data dir, cache dir) and gave third-party spawns a
  stricter policy that strips even the helper keep-set.

So the remaining hole is narrow and deliberate: **agent panes, and everything
they launch, hold a full-authority bearer token.**

## 3. Threat model — stated honestly

An agent is *semi-trusted*: it runs arbitrary code by design, and this spec does
not pretend to contain a hostile agent. The goals are narrower and achievable:

1. **Blast radius.** A postinstall script or a language server launched from a
   pane should not inherit full API authority as a side effect of where it was
   started.
2. **Cross-instance.** A credential inherited by a *different* AgentMux build
   is authority over the instance that spawned it — the confused-deputy case,
   and the one with no legitimate use.
3. **Auditability.** Today every call is indistinguishable; the server cannot
   say which pane acted.

Non-goal: preventing a determined agent from calling the API it is *meant* to
call.

## 4. Options

### A. Status quo
Zero work. Leaves every descendant holding full authority, and the
`credential.rs` gate remains a one-off rather than a pattern.

### B. Second factor on sensitive surfaces (extend `credential.rs`'s approach)
Classify routes; require a host-only secret for the dangerous ones. Cheap,
incremental, proven in-tree. But it is a denylist — each new sensitive route must
be remembered, which is the failure mode that produced five review rounds on
#3326.

### C. Scope and expire the token
Mint a per-pane token carrying `block_id` + capabilities + a short TTL, instead
of handing out the instance-wide key. The server then knows *which pane* called
and can refuse out-of-scope methods. Inheritance still leaks something, but it
leaks a narrow, expiring capability rather than the master key.

### D. Per-invocation handoff over a Unix socket (**recommended**)
No credential in the environment at all. Each instance exposes a socket under
its own runtime dir; helpers connect and are authorised by **peer credentials**
(`SO_PEERCRED` / `LOCAL_PEERCRED`), which the kernel supplies and a process
cannot forge or inherit.

Authority then comes from *who is connecting*, not from a string anyone
downstream can read. An inherited environment grants nothing, because there is
nothing in it.

There is precedent: the launcher already uses a per-instance Unix socket
(`/run/user/<uid>/agentmux/<hash>.sock`), and socket path discovery is a solved
problem here now that `authkey.dev` publishes per-instance endpoints.

Costs: Windows needs a different mechanism (named pipe + `GetNamedPipeClientProcessId`),
and `SO_PEERCRED` identifies a *process*, not a pane — so mapping pid → pane
still needs a registry, and a pane's children share its authority unless
combined with (C).

### E. File-descriptor passing
Strictly stronger — hand the helper an already-authenticated fd it cannot
re-derive. Rejected for now: it requires every helper to be launched by us, and
`muxsh` is a shell function the user may invoke at any time.

## 5. Recommendation

Phased, because D alone is a redesign of four helper CLIs:

1. **Now — stop the bleeding.** Strip `AGENTMUX_AUTH_KEY` (and
   `AGENTMUX_LOCAL_URL`) from *third-party* spawns. Largely landed in
   `SPEC_PANE_ENV_ISOLATION`'s strict policy; audit that no new spawn path
   reintroduces it. Low risk, no helper changes.
2. **Next — (C).** Per-pane, expiring, scoped tokens. This is the single
   highest value-to-risk step: it preserves the current env-var transport, so
   `lib/muxclient.mjs` keeps working unchanged, while turning "the master key"
   into "a capability for this pane". It also delivers the auditability in §3.3.
3. **Then — (D)** for helpers, with the token from (2) as the fallback for
   anything not launched through the socket. Peer-credential auth removes the
   secret from the environment entirely.
4. **Fold `credential.rs`'s second factor into the general model** once (C)
   exists, so the host-only gate becomes "a capability agents are never
   granted" rather than a bespoke check.

## 6. Why not just rotate or shorten the key

Rotation does not help: the leak is *inheritance at spawn*, so a child gets
whatever is current. A short TTL without scoping just breaks long-running agents.
Scope is the property that matters; expiry is only useful alongside it.

## 7. Open questions

- **Does anything depend on a pane reaching the API as the *instance* rather
  than as itself?** `muxspect`'s cross-channel lookup queries other channels —
  (C) must not break that, or it needs an explicit capability.
- **Windows parity.** Named-pipe peer identity is well-defined, but the code has
  no precedent for it; (D) should not land Unix-only.
- **MCP.** `agentmux-mcp` reads the key from its own environment too. It is
  spawned by us, so it is a good first consumer of (D) — but it must be migrated
  in step with the helpers, not before.
