# REPORT — container agents could never authenticate, and always started with an empty /workspace

**Date:** 2026-09-02
**Author:** Agent5
**Status:** implemented — fix shipped in PR #2931. Diagnosed live against
AgentMux v0.55.31 and `ghcr.io/agentmuxai/agent-claude:latest`.
**Severity:** P0 for container agents — the feature has never worked end to end.
No operator action (re-login, rebind, restart) could fix it.

---

## 1. Symptom

A container agent answers every message, however it is delivered, with:

```
Not logged in · Please run /login
```

The operator logs in again through Armory, the login **succeeds**, and nothing
changes. The agent's own transcript shows a turn that really ran and then failed
in 59 ms:

```json
{"type":"system","subtype":"init","cwd":"/workspace","apiKeySource":"none", …}
{"type":"assistant","message":{"content":[{"type":"text",
  "text":"Not logged in · Please run /login"}]},"error":"authentication_failed"}
```

Note `apiKeySource: "none"`. The CLI started fine; it simply had no credential.

---

## 2. Root cause: credentials are provisioned to the host, never to the container

### 2.1 The two ends never meet

The Armory login writes the account's credentials to its isolated host dir:

```
~/.agentmux/channels/<channel>/identities/<account>/claude/
  .credentials.json    509 B     ← the OAuth token, written by the login
  .claude.json
  CLAUDE.md
  projects -> …/shared/identities/<account>/claude/projects   (symlink)
```

The container's config dir is a different thing entirely — an empty per-agent
named volume:

```
/home/agent/.claude              (volume agentmux-claude-agentmux-<agent-id>)
  .claude.json    363 B          ← CLI scratch, no tokens
  backups/ projects/ sessions/
  (no .credentials.json)
```

### 2.2 Why nothing bridges them

1. The identity resolver resolves the account and injects `CLAUDE_CONFIG_DIR`
   into the spawn env. This part works — it logs
   `injected CLAUDE_CONFIG_DIR for oauth provider claude`.
2. `CLAUDE_CONFIG_DIR` is listed in `CONTAINER_ENV_DENYLIST` (`container.rs`), so
   the container spawn **strips it** before exec. That is correct on its own: it
   is a Windows host path and means nothing inside a Linux container.
3. The container falls back to the image's own
   `CLAUDE_CONFIG_DIR=/home/agent/.claude` — the empty volume.
4. **No code anywhere copies or mounts credentials into a container.** `grep
   credentials.json` across `agentmux-srv` matches only host-side `identity/`
   and `server/` files; never `container.rs` or `container_spawn.rs`.

The mount code states the design intent that produced the gap:

> *"Using a named volume (not a host bind mount) avoids credential leakage from
> the host's .claude directory into the container."*

Isolation was implemented. **Provisioning was not.** This is
`SPEC_CONTAINER_PANE_SUPPORT_2026_06_11.md`'s Open Question #2 — *"Credential
provisioning: who provides the credential to the container? Decision needed
before Phase 2"* — which was never answered for the OAuth path.

### 2.3 Why re-login cannot help

Every login writes to the host dir in §2.1. Nothing reads it from inside the
container. The loop is closed: the operator can repeat the login indefinitely
and the container's view never changes.

---

## 3. Second, independent defect: `/workspace` is empty

The running container had exactly one mount — the credentials volume.
`/workspace` was empty and root-owned. `ensure_running_locked` mounted the
`.claude` volume plus whatever the caller passed in `agent:container_volumes`
(default `[]`); the spec's *"workspace mounted automatically from
working_directory"* was never implemented. So even with authentication fixed, an
agent would have had no files to work on.

---

## 4. The fix, and the two shapes that do not work

The obvious fix — bind-mount the account's whole config dir over
`/home/agent/.claude` — **does not work.** Both failure modes were verified live
rather than reasoned about:

1. **The `projects` symlink does not resolve in-container.** It points into the
   shared identities tree, which Docker surfaces as
   `/mnt/host/c/Users/...`; that path does not exist in the agent image.
   `ls` → `No such file or directory`.
2. **A nested volume cannot shadow it.** Mounting the per-agent volume at
   `/home/agent/.claude/projects` to cover the broken link **silently does not
   mount at all** — Docker will not mount over a path that is a symlink inside a
   bind mount. `mount` inside the container showed only the parent 9p bind, and
   the symlink was still there. Chowning does not help: there is nothing to
   chown. (A first attempt also hit `Permission denied` on that path, which is
   the same failure wearing a different hat — the volume was never there.)

### 4.1 What ships instead

Keep the per-agent named volume at `/home/agent/.claude` exactly as before, and
add **one** mount: the account's `.credentials.json`, bind-mounted as a single
file on top of the volume.

This avoids both failures above and preserves the existing ownership story — the
volume is still initialised from the image, so `projects`/`sessions` remain
writable by uid 1000. Plus the workspace bind that §3 was missing.

Verified end to end: a container using this exact mount shape completed a real
turn against the bound account.

```json
{"type":"result","subtype":"success","is_error":false,"result":"AUTH_OK"}
```

### 4.2 Existing containers must be recreated

`ensure_running` was idempotent to a fault: a *running* container short-circuited
before any mount was considered. Docker cannot add a mount to an existing
container, so the fix would have reached no agent whose container was already up
— which is every agent that has ever run.

`mounts_match` now compares the desired home-dir and workspace mounts against
the live container and recreates on drift. It compares only the mounts this
module owns, never the user's own `container_volumes`, and **fails safe**: any
inspect error is treated as "matches", because a spurious recreate kills a live
agent and loses its container-local state, which is worse than one more restart
on a stale mount.

That comparison must run in **both** directions — a mount that should now be
ABSENT is drift too. The first cut checked only that the desired mounts were
present, which left a security hole (reagent P1 on PR #2933): unbinding an
agent's Armory account is a normal transition that never reaches
`SpawnGateError::MissingCredentials` — that gate fires when credentials are
expected and missing, not when an agent legitimately has none. With a
presence-only check, `agent_home_mounts` would stop asking for a credentials
mount while the running container kept the old account's `.credentials.json`
bind-mounted and writable indefinitely, so every later turn would keep
authenticating, and refreshing tokens, as the account the operator had just
unbound. The owned target set is now compared exactly, so unbinding forces a
recreate.

---

## 5. Known limitation

A single-file bind follows the inode. If the CLI ever replaces
`.credentials.json` by atomic rename rather than writing in place, a refreshed
token would land on the host but not be seen by the running container until it
is recreated. This is recorded rather than fixed: the read path — the case that
was totally broken — is unaffected, and the write-back path cannot be verified
without waiting out a real token expiry. If refresh-after-expiry turns out to
break in a long-running container, this is the first thing to suspect.

---

## 6. What this does not change

- The credential spawn gate (`SpawnGateError::MissingCredentials`). It behaves
  exactly as `SPEC_ACCOUNT_DELETE_DEAUTH_LAYERS_2_4_2026_07_14.md` §2.2
  specifies. It was a separate, earlier symptom in the same investigation and
  was resolved by the operator's re-login.
- `CONTAINER_ENV_DENYLIST`. Stripping a host `CLAUDE_CONFIG_DIR` is right; the
  fix works *with* that, by mounting rather than by forwarding an env var.
- Agents with no bound OAuth account, which keep the previous
  named-volume-only shape byte for byte.

---

## 7. References

- `agentmux-srv/src/backend/container.rs` — `ContainerMountSpec`,
  `agent_home_mounts`, `mounts_match`, `CONTAINER_ENV_DENYLIST`
- `agentmux-srv/src/server/agent_handlers/input.rs` — `agentinput` container branch
- `agentmux-srv/src/server/app_api/agent_io.rs` — `agent.send` container branch
- `docs/specs/SPEC_CONTAINER_PANE_SUPPORT_2026_06_11.md` — Open Question #2
- `docs/specs/SPEC_HOST_VS_CONTAINER_AGENTS_2026_06_18.md` — container-agent UX,
  still open and unrelated to this fix
- `docs/reports/REPORT_JEKT_DELIVERY_DROPS_SUBPROCESS_AGENTS_2026_09_02.md` — the
  delivery-path bug found earlier in the same investigation (PR #2930)
