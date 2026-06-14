# Retro: Per-build launch isolation — completing the #1315 fix (2026-06-13)

**Author:** AgentX
**Triggered by:** User observation — launching a freshly-built `task package` portable joined a still-running build of the same branch (and, when launched from inside an agent pane, adopted the *parent* instance's channel) instead of starting as its own instance.

---

## 1. What happened

A local portable built from this branch, launched to smoke-test a fix, did not run its own binary. Two distinct failure modes:

1. **Standalone, same branch:** a second build of the same branch joined the first build's running window — Chromium logged `CEF early exit (process singleton or similar) — exiting cleanly exit_code=24` / `Opening in existing browser session`.
2. **Nested inside another AgentMux:** launched from an agent terminal, the build adopted the *parent* pane's channel (`local-agentx-fix-term-scrollbar-c…`) instead of its own baked channel, then collided as in (1).

## 2. Root cause — three isolation keys that disagreed

| Key | Protects | Keyed on (before this fix) |
|---|---|---|
| Data dir | SQLite, config | `(channel, semver)` → per-**branch** |
| Single-instance pipe | "another launcher up?" | `(data_dir, build label)` → per-**build** |
| CEF user-data-dir singleton | "another browser on this cache?" | cef-cache = `(channel, semver)` → per-**branch** |

**#1315** ("fix per-build pipe isolation", retro `retro-local-build-isolation-regression-2026-06-09.md`) made the *pipe* per-build via `AGENTMUX_BUILD_LABEL`, and declared the problem solved. But it only moved the collision down a layer: the pipe was now per-build, yet the **cef-cache stayed per-branch**, so a second build's host still hit Chromium's user-data-dir singleton and forwarded into the first. The retro's own §7.2 workaround — *"close all running instances first"* — is the tell that concurrent isolation was never actually achieved.

The build ID (`AGENTMUX_BUILD_LABEL`) reached the pipe but never the **data-dir identity** (the channel), so cef-cache (derived from the channel at `data_paths.rs:209`) and the data dir stayed per-branch.

Separately, **portable builds honor an inherited `AGENTMUX_CHANNEL`** (`data_dir.rs::resolve_paths` → `CommonDataPaths::resolve`), while dev builds use `resolve_path_only` to ignore it. A portable launched inside another AgentMux pane inherits the parent's `AGENTMUX_CHANNEL` (set with `AGENTMUX=1` and the full path env by `blockcontroller/shell.rs` for every pane shell) and adopts the parent's data dir.

## 3. Why #1315 couldn't go further (and why we can now)

#1315 deliberately kept the data dir per-branch to preserve "session persistence — agents, auth, panes survive a rebuild" (its retro §3). Putting the build ID in the data dir then would have dropped the user's agents and forced a re-login on every rebuild.

The **cross-channel work (#1387–#1393, shipped 2026-06-13)** moved agents (definitions, instances/"My Agents", workspaces, transcripts, `--resume` session_id) **and auth** (oauth/accounts/identities — credentials live under `shared/` + `~/.claude`, identity DB rows rebuilt from them on startup) to **global** stores. So a per-build data dir now keeps every agent and stays logged in. The blocker is gone.

## 4. The fix

1. **`scripts/package.sh`:** bake a per-build `BUILD_ID` (8-char hash of the full label) into `AGENTMUX_BUILD_CHANNEL_DEFAULT` → channel `local-<slug>-<hash>-<build-id>`. cef-cache + data dir + pipe are now all per-build. (Releases unaffected — `RELEASE_CHANNEL` still wins; ≤55/64 chars.)
2. **`agentmux-launcher/src/data_dir.rs`:** when launched **nested** (`AGENTMUX` set), resolve path-only and ignore the leaked ambient `AGENTMUX_CHANNEL` — symmetric with dev builds. The launcher's resolved env is already authoritative for the host+srv (`to_env_vars()` overwrites inherited values at every spawn site), so no downstream change is needed. An explicit *standalone* override is still honored.
3. **GC** in `package.sh`: keep the newest `AGENTMUX_LOCAL_CHANNELS_KEEP` (default 5) per-build channels per branch, prune older (skipping anything touched in the last 30 min) so disk doesn't grow unbounded.

## 5. Isolation invariants (I1–I6)

Verified the change **strengthens** I1 (pipe uniqueness), I4 (forward-only contact), and I6 (data isolation — now per-build, a strict refinement); I2/I3/I5 untouched. Two distinct builds of one branch running simultaneously is provably safe — separate SQLite, pipe, cef-cache, and unnamed job objects.

## 6. Costs / follow-ups

- **Memories (`db_memory_bundles`) and pane/tab layout do not carry per-build** — they were never globalized. Consistent with how they already don't cross *branches*. Globalizing memories (the same treatment agents got) is a follow-up if cross-build memory matters.
- **Disk accumulation** is bounded by the GC, not eliminated; `AGENTMUX_LOCAL_CHANNELS_KEEP` tunes it.

## 7. Lessons

- **A per-build identity that doesn't reach *every* isolation key is only half-applied.** #1315 wired the build ID into the pipe but not the cef-cache/data dir; the CEF singleton silently re-coupled what the pipe split. When isolating, enumerate *all* the keys (pipe, data dir, cef-cache) and make them agree.
- **The "close all instances first" workaround in a retro is a smell** that the isolation isn't real — it means a second concurrent instance still collides somewhere.
- **Env leaks cross the process boundary.** `AGENTMUX_CHANNEL` set for a pane shell silently redirects anything launched from it; portable builds must treat an inherited channel as ambient noise (use `AGENTMUX=1` to detect nesting), exactly as dev builds already do.
