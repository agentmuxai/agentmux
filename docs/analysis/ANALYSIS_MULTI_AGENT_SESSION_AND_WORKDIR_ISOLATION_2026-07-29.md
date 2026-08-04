# Analysis: multi-agent session ownership + working-directory isolation (host, sandbox, LAN/WAN)

**Date:** 2026-07-29
**Status:** Discussion / analysis — no code change yet. Written to seed a fresh planning conversation, not as a committed plan.
**Prompted by:** `docs/retros/RETRO_DEV_BUILD_SHARED_AGENT_SESSION_COLLISION_2026_07_29.md` — a throwaway `task dev` build resumed a live production agent session and spawned an independent second process that ran real `git checkout` in the same physical working directory as the first.

## TL;DR

Two separate gaps, neither covered by the existing isolation contract:

1. **Working-directory collision.** Concurrent agents (same machine) sharing one git checkout can race on `git checkout`/`stash`/`reset`. `git worktree` is the obvious fix but has a real-world tangling reputation; **per-task `git clone --reference <primary>` is the recommended alternative** — independent working tree/index/HEAD, deduped object store, and it composes for free with existing `clone_id`-based data isolation (below).
2. **Session/turn ownership.** Agent identity, definitions, and conversation transcripts resolve via a **global**, channel/instance-independent root (`~/.agentmux/shared/agents/{registry,definitions,transcripts}`) — by design, so a named agent's history follows you across builds. Nothing stops two live processes from both resuming and driving the same session's next turn concurrently. This is the actual root cause of the retro incident.

Both gaps need to scale across host agents, sandbox/container agents, and eventually LAN/WAN-connected hosts — the recommendation below is chosen specifically because it doesn't need a redesign at each of those boundaries.

---

## 1. Working-directory isolation

### 1.1 What's already solved

`ANALYSIS_MULTI_CLONE_TASK_DEV_ISOLATION_2026-05-26.md` (shipped) already isolates **local app state** per clone: `RuntimeMode::Dev`'s `clone_id` is a hash of the clone's canonical workspace-root path (`agentmux-common/src/runtime_mode.rs::derive_clone_id`), so `~/.agentmux/dev/<branch>/<clone_id>/` is automatically distinct for any two clones at different paths — confirmed by the existing `derive_clone_id_differs_between_clones` test, no configuration required. Two clones of this repo already get fully separate data dirs, lockfiles, and named-pipe IPC (I1/I5/I6 in CLAUDE.md's isolation invariants).

### 1.2 What's not solved

That mechanism isolates **AgentMux's own state**, not the **git working tree itself**. Nothing stops two agent processes from being pointed at the *same* physical checkout and running `git checkout`/`commit`/`stash` concurrently — which is exactly what happened in the retro (reflog showed a second process's `git checkout -b` landing mid-session in this session's own working directory).

### 1.3 Recommendation: clones over worktrees

Worktrees were considered and rejected based on prior firsthand experience: they get tangled (stale worktree entries after manual deletion, "already checked out elsewhere" errors, shared-index edge cases). Recommendation is **per-task `git clone --reference <primary-clone-path> <task-dir>`**:

- Independent working tree, index, and `HEAD` per task — no shared mutable git state, so the collision class in the retro can't happen.
- `--reference` (without `--dissociate`) dedupes the object store against the primary clone — cheap and fast, not a full duplicate.
- Composes for free with the existing `clone_id` mechanism (§1.1): each cloned task directory is a distinct canonical path, so AgentMux's own data-dir isolation falls out automatically with zero new code.

Tradeoff: each clone needs its own build (`target/`, `node_modules/`, `dist/cef-dev/`) unless build outputs are cached/shared explicitly (e.g. `CARGO_TARGET_DIR` pointed at a shared cache, matching the "could be optimized with sccache" note already flagged as out-of-scope in the 2026-05-26 analysis). Worth a follow-up if per-task clone cost becomes painful in practice.

### 1.4 Host vs. sandbox agents

- **Host agents**: use the clone-per-task approach above directly against the host filesystem.
- **Sandbox/container agents**: containers already have their own filesystem namespace, which *should* make this a non-issue — but only if each container gets its own clone rather than all containers bind-mounting the same host directory (which reintroduces the identical hazard, just inside containers). Two ways to get a per-container clone: (a) a host-side clone per container, bind-mounted in, or (b) `git clone` *from a shared bare repo* at container-start, run inside the container — avoids host-side disk duplication entirely and is arguably the more natural fit given containers are already ephemeral. **Needs verification**: which of these AgentMux's current `ContainerManager` does today (see `agentmux-srv/src/backend/container.rs`) — if it bind-mounts one shared host path into multiple containers, that's the same collision wearing a container costume, worth its own follow-up regardless of the session-lease work.

### 1.5 LAN/WAN hosts

Once agents run on physically separate machines, working-directory collision stops being possible by construction — there's no shared filesystem to race on. Git itself (push/pull) becomes the sync mechanism. This isolation problem doesn't extend to LAN/WAN; only §2 does.

---

## 2. Session/turn ownership

### 2.1 Root cause (from the retro, restated)

`agentmux-srv/src/registry/paths.rs::resolve_global_shared_root()` deliberately resolves to a single, OS-home-scoped `~/.agentmux/shared/` regardless of channel/`RuntimeMode`/`clone_id` — intentional, per its own doc comments, so a named agent and its history follow the user across builds/channels. Resuming a session (`agent_open.rs`) seeds `resume_session_id` from that shared registry whenever no live controller exists *in the current process* — it has no way to know a different process, anywhere, already owns that session live. Submitting a turn then spawns a new local child process (`blockcontroller/core.rs`, `tokio::process::Command`) with no cross-process coordination at all.

This isn't covered by CLAUDE.md's existing isolation invariants (I1–I6): those govern OS-level object naming and data-dir separation between *distinct* `(channel, version)` pairs. The incident involved two processes that were *already* correctly data-isolated per I6 — the shared registry is a deliberate exception to that isolation, and nothing plays the role I4 ("forward-only cross-instance contact... side-effect-free") plays for pipe-based window forwarding. There's no I7 for "at most one live process may drive a given session's next turn."

### 2.2 Recommendation: one lease primitive, pluggable backend

Design a single claim/renew/release lease interface now, rather than a local-only quick fix that needs a rewrite once real multi-host distribution shows up:

- **Interface**: `claim(session_id) -> Result<Lease, AlreadyHeld>`, `renew(lease)`, `release(lease)`. A lease carries an owner id + heartbeat expiry.
- **Local backend** (single machine, offline): a row in the existing shared SQLite registry, claimed via a short transaction / advisory lock, heartbeat-renewed while a turn is in flight, expiry reclaims a crashed owner's lease.
- **Cloud backend** (LAN/WAN, once connected): the existing muxbus layer is the natural fit — it's already the cross-host trust/identity boundary (per-agent credentials already flow through it, per this session's earlier `feat(muxbus): per-agent M2M credential fetch/cache` work), so extending it with claim/renew/release RPCs reuses an existing trust relationship instead of inventing a new one.
- Same interface both ways means single-machine correctness ships first without a second design pass later for the distributed case.

**Open question, not resolved here**: should read-only multi-window viewing of a live session remain unrestricted (probably yes — only turn-*driving* needs exclusivity), and what UX signals a refused claim (dev-mode picker showing "live elsewhere" per the retro's follow-up, vs. a hard error)?

### 2.3 Host vs. sandbox agents

No difference in principle — the lease is about the *session*, not the process's execution environment. A sandboxed agent claims/renews/releases the same way a host agent does.

### 2.4 LAN/WAN hosts

This is where the lease mechanism actually matters most, per §1.5 — once working-directory collision is impossible by construction, session ownership is the only remaining coordination surface, and it's the one that has to work correctly across hosts, not just processes on one machine.

---

## 3. Suggested next concrete step

Sketch the lease interface (§2.2) in more detail — schema for the registry row, exact claim/renew/release semantics, expiry timing, and the refusal UX — before touching host/sandbox filesystem specifics, since §1's recommendation (clone-per-task) is independent of it and could ship first without waiting.

## References

- `docs/retros/RETRO_DEV_BUILD_SHARED_AGENT_SESSION_COLLISION_2026_07_29.md` — the incident this analysis follows up on, including reflog evidence of the working-directory collision.
- `docs/analysis/ANALYSIS_MULTI_CLONE_TASK_DEV_ISOLATION_2026-05-26.md` — the shipped `clone_id` data-dir isolation this builds on.
- `agentmux-common/src/runtime_mode.rs` — `RuntimeMode::Dev`, `derive_clone_id`.
- `agentmux-srv/src/registry/paths.rs` — `resolve_global_shared_root`, `resolve_shared_registry_dir`, `resolve_shared_definitions_dir`.
- `agentmux-srv/src/server/agent_handlers/session.rs` — `COMMAND_LIST_RECENT_SESSIONS`.
- `agentmux-srv/src/server/app_api/agent_open.rs` — session resume seeding.
- `agentmux-srv/src/backend/blockcontroller/core.rs` — local child-process spawn on turn submission.
- `agentmux-srv/src/backend/container.rs` — `ContainerManager`, referenced in §1.4 as needing verification for per-container clone vs. shared bind-mount.
- `agentmux-srv/src/muxbus/` — existing cross-host trust/identity layer proposed as the cloud lease backend in §2.2.
- CLAUDE.md `### Multiple Instances Run in Parallel` — the existing I1–I6 isolation invariant contract this analysis identifies a gap in (no invariant currently governs session/turn ownership).
