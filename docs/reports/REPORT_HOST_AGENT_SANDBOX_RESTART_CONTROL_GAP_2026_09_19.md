# Report: a host agent has no safe way to restart a sandbox agent — the only reachable path destroys its container, its conversation, and any config fix in flight

**Date:** 2026-09-19
**Status:** investigated, not yet implemented — this documents the gap, it does not close it
**Author:** Camper
**Repo state:** main @ `ffdfa4aa3` ("fix(container): ship agentmux-mcp/agentmux-bashwrap in the sandbox image (#2939)", PR #3427)
**Probed live** against a running instance (`narko`), with a real container-type
agent (`Mazop`) as the subject
**Related:** `docs/reports/REPORT_AGENT_OPEN_API_GAP_2026_09_06.md` (already
flagged container agents — #2939/#1400 — as blocked "whenever a GUI is
unavailable"; this report is the concrete follow-through, from the other side:
what happens once an agent *can* be opened, but needs to be safely restarted),
`docs/specs/SPEC_AGENT_PANE_LIFECYCLE_CONTROL_2026_09_10.md` (`ClosePane`'s own
spec), `docs/specs/SPEC_AGENT_PANE_CLOSE_GRACEFUL_SHUTDOWN_2026_09_18.md`
**Blocks:** any host agent trying to remotely fix a container agent's
environment (this session's own motivating case) or supervise/restart one
programmatically (Warden-style watchers, `SupervisorNudge`'s broader family)

---

## 1. The question, and the answer

**Asked:** given a host agent (this session, "Camper") needs to fix another
agent's ("Mazop", container-type) broken MCP configuration and have it take
effect, what App API sequence does that safely?

**Answer: none exists.** `ClosePane` + `OpenAgent` is the only reachable
sequence, and it does three things nobody asked for, none of them documented
at the tool-description level:

1. **Destroys and recreates the container**, not merely restarts a process
   inside it.
2. **Discards the conversation**, despite `ClosePane`'s own description
   implying otherwise.
3. **Loses any config edit made after the container was created**, because
   config write and process spawn happen inside one atomic `agent.open` call
   with no host-reachable hook in between, and — separately — the write is
   unconditional, overwriting a manual fix even on a *later* call.

Each is demonstrated below against a real running container, not inferred from
reading code alone.

## 2. Container identity does not survive a restart

| Before restart | After `ClosePane` + `OpenAgent` |
|---|---|
| Container hostname `a63adf3e2922` (confirmed via `hostname`, per Mazop's own earlier report) | Container hostname `a7e8a9198dcc` (confirmed via `docker exec <id> hostname`, run directly against the live host) |
| `docker ps -a` showed one container for this agent | Same — the old one is gone entirely, not stopped-and-kept. `docker ps -a` after the restart shows no `Exited` container matching Mazop's `definition_id` (`a0118d9a-ad16-42f0-8329-000dae02bab3`) anywhere in the list; every other `Exited` entry belongs to unrelated agents, weeks old. |

This resolves an apparent contradiction in the code: `container.rs` has both a
`find_container` lookup (by name, `list_containers(all: true)` — would see a
*stopped* container too) and a `stop`/`remove` pair, which reads like it
*could* support reuse. Empirically, it doesn't reuse on this path — the old
container is gone, not paused. Whether `find_container` is even reached before
a fresh `docker run`/`create` on the `agent.open` path, or the close saga
removes rather than stops a container-type block, wasn't traced line-by-line
here (the observed *outcome* — new hostname, no leftover stopped container —
is what this report is based on); either way, the outcome for a caller is the
same: **`ClosePane` on a container agent is a delete, not a pause.**

This matters beyond aesthetics: `SPEC_IAM_ROLES_ANYWHERE_MIGRATION_2026_09_18.md`
§2 deliberately grains Roles Anywhere certificates **per-container** for
exactly this agent type, on the reasoning that "a container is a real,
enforced isolation boundary." That design assumed a container's identity is
stable for its lifetime. It is not — every `ClosePane`/`OpenAgent` cycle
produces a **new** container needing (by that spec's own logic) a **new**
certificate, silently. Mazop's cert issued for `CN=a63adf3e2922` still
authenticates fine after this restart (Roles Anywhere validates the cert
against the CA, not against a live hostname check), but its CN is now a
description of a container that no longer exists — a latent audit/attribution
problem, not (yet) an availability one.

## 3. Conversation history does not survive a restart

`ClosePane`'s own tool description states: *"Closing a pane only removes it
from the layout; the underlying agent's conversation history is not deleted."*

Literally true — the transcript file isn't deleted. But nothing in the
`OpenAgent` path resumes it once the pane is fully closed. `agent_open.rs`
only populates `resume_session_id` when `agent_live_elsewhere` is true —
i.e., when the *same* agent is detected running concurrently in another pane
(the two-controllers-sharing-one-session-id scenario the surrounding comments
describe). That flag is false once the only pane has already been closed, so
a plain `ClosePane` → `OpenAgent` sequence starts a **cold** session.

Observed directly: after the restart, Mazop's first message was *"No prior
memory. I'm Mazop, ready to help. What would you like to work on?"* — it had
to be re-briefed from scratch on everything from this session's file-bridge
exchange (see `jekt-to-camper.md`/`camper-to-mazop.md` in its own workspace),
none of which it could see itself, mid-restart, without knowing to look.

There is no tool-level way to ask for "close, but keep the resumable session
id" — the resume mechanism exists in the code but is wired to a different
trigger than "I, the caller, want this restart to resume."

## 4. Any config fix a host agent makes is racing a write it cannot see or win

This is the most consequential finding for anyone trying to do what this
session was trying to do: fix a container agent's `.mcp.json` from outside.

- `agentmux-srv/src/backend/agent_config.rs:231` and `:299` hardcode
  `"command": "agentmux-mcp"` — a bare `PATH` lookup, no container-vs-host
  branch anywhere in either `build_mcp_config` or `build_mcp_config_from_refs`.
- `agentmux-srv/src/server/app_api/agent_open.rs`, around the
  `write_agent_config_files` call site, carries its own comment stating the
  behavior plainly: *"No collision resolution in this path — the function
  creates the dir if missing and overwrites whatever's there."* — i.e. this
  is not accidental, it's documented-as-designed to always overwrite.

Consequence, demonstrated this session: I hand-edited the live `.mcp.json` on
the shared host mount to point `command` at an absolute path (the container
image doesn't have `agentmux-mcp` on `PATH` at all yet — a separate, now-fixed
gap, PR #3427) and to add `LD_LIBRARY_PATH`. Calling `OpenAgent` **silently
reverted both edits** back to the hardcoded template before the new process
ever read the file. I re-applied the fix immediately after — too late; the
process had already attempted (and failed) its one-shot MCP connection by
then, confirmed in that session's own `system/init` transcript line
(`"agentmux","status":"failed"`).

There is no host-reachable ordering here that wins: config write and process
spawn happen inside one `agent.open` call, and the write itself is
unconditional. A host agent cannot inject a persistent override (there is no
merge/overlay mechanism for the `agentmux` server entry specifically — other
entries like `context7`/`fetch`/`git`/`memory`/`playwright` appear to come
from a template that's merged in wholesale each time, but the `agentmux`
entry's `command` and `env` keys are always the hardcoded/regenerated values,
with no documented slot for a caller-supplied addition to survive).

**Net effect: a second restart would not have fixed anything either.** It
would have destroyed the (already-recreated) container again, discarded the
(already-cold) session again, and regenerated the same broken config again.
Recognizing this stopped further restart attempts this session — each one
costs a real provider-token spend (`OpenAgent`'s own description: "consumes
provider tokens like any human-opened pane") for a predictably identical
outcome.

## 5. What's actually missing

1. **A restart verb distinct from delete+recreate.** For a container-type
   agent, "restart" should mean stop-and-start (or exec a fresh process into
   the *same* container), preserving container identity (§2) — not
   `docker run` from scratch. If destroy+recreate is sometimes the right
   behavior (e.g. picking up a new image), it should be a distinct, explicit
   action from "restart the process," not the only option available under
   either name.
2. **A resume path triggered by intent, not by concurrency detection.**
   `resume_session_id` already exists and works for the "agent live
   elsewhere" case; it needs a second trigger — "the caller explicitly asked
   to restart and wants the prior session resumed" — that doesn't depend on
   a stale pane still being technically open somewhere.
3. **A supported override point for per-agent MCP config**, at least for the
   `agentmux` server entry itself. Every other entry in the generated
   `.mcp.json` looks template-sourced already; the `agentmux` entry has no
   equivalent seam. Even a narrow one (e.g. an optional
   `AGENTMUX_MCP_BINARY_PATH` env var honored by `build_mcp_config`, read
   once at the same point `AGENTMUX_JEKT_KEY` etc. get injected) would have
   made this session's fix durable across a restart instead of racing one.
4. **Tool-description accuracy.** `ClosePane`'s description ("conversation
   history is not deleted") is true but misleading for the container-agent
   case investigated here, where the practical experience is total state
   loss. `OpenAgent`'s description already correctly notes it "spawns a real
   provider process" — it should also say plainly, for container agents,
   that this means a **new container**, not a resumed one, until finding #1
   above is addressed.
5. **A "recreate from latest image" action**, separate from a same-image
   restart. This session's actual unblock for Mazop (PR #3427, already
   merged) requires exactly this — the fix is real and shipped, but nothing
   in the reachable API lets a host agent (or a human) request "give this
   agent a container built from the current image" on demand; it happens
   only incidentally, as an undocumented side effect of any `ClosePane` +
   `OpenAgent` cycle, bundled with all the losses in §§2–4.

## 6. What this report does not claim

This is not a claim that destroy+recreate is the wrong default forever, or
that today's behavior is a bug rather than an intentional simplicity
tradeoff — it may well be exactly that, undocumented. It's a claim that the
tradeoff is currently invisible at the tool-call layer: nothing in
`OpenAgent`'s or `ClosePane`'s description, and nothing this session could
find exposed to an agent short of reading `agentmux-srv`'s own source and
running live `docker` commands against the host, would have predicted any of
§§2–4 in advance. A host agent attempting the same fix this session attempted,
without also having Bash/Docker access to the underlying host, would have no
way to discover the config-write race at all — it would just observe the fix
"not sticking," with no visible reason why.
