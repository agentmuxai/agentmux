# SPEC: Unified Agent Conversation-History Store

**Date:** 2026-06-10
**Status:** Draft / proposal
**Author:** AgentA
**Related:** `agentmux-srv/src/backend/history/claude_adapter.rs`, `agentmux-srv/src/backend/agent_session.rs`, `agentmux-srv/src/backend/providers.rs`, `agentmux-common/src/data_paths.rs`, `agentmux-srv/src/server/app_api.rs`, `scripts/import-agents.sh`

---

## 1. Problem

Conversation histories are produced and stored by each **provider CLI** (Claude Code, Codex, Gemini, Kimi, ACP providers…), each in its own on-disk layout. AgentMux does not own, unify, organize, or expose them. Symptoms:

- A stopped or **imported** agent shows an **empty pane** — its prior conversation is not reconstructable by AgentMux.
- The same logical agent's history **splinters** across many buckets (different working dirs, different portable-build paths).
- There is **no way to browse, search, label, or clear** conversation history by agent.
- Pre-isolation histories are **orphaned** in the user's global provider home, invisible to current builds.

**Requirement (user):** entire conversation histories must be stored inside AgentMux's home, unified across all providers, with the ability to **organize and clear** them, so users always have access to full history.

---

## 2. Verified current state (2026-06-10)

### 2.1 Provider history locations (filesystem-verified)

| Provider | Native history location | Format | Resume |
|----------|-------------------------|--------|--------|
| **claude** | `$CLAUDE_CONFIG_DIR/projects/<cwd-slug>/<session_id>.jsonl` | NDJSON (assistant/user/tool/attachment records) | `--resume <sid>` |
| **codex** | `$CODEX_HOME/sessions/<YYYY>/<MM>/<DD>/rollout-<ts>-<id>.jsonl` | NDJSON (`session_meta` + events) | `exec resume <id>` |
| **gemini** | `$GEMINI_CLI_HOME/history/<id>/…` | per-project dir | `-r <sid>` |
| **kimi** | `$KIMI_SHARE_DIR/sessions/`, `…/user-history/` | sessions | none (native) |
| **qwen** | `$QWEN_HOME/…` | stream-json | none |
| **openclaw / pi / copilot** | `$*_HOME/…` (ACP-native) | protocol-managed | ACP session, no flag |

### 2.2 AgentMux already isolates provider homes — **inside its own home**

At spawn time (`app_api.rs` ~1500–1560) AgentMux sets each provider's config/home env var to:

```
~/.agentmux/shared/providers/<auth_dir_name>/        # account-wide, channel/version-independent
# or, when an identity bundle is active:
~/.agentmux/shared/identities/<bundle_id>/<auth_dir_name>/
```

`DataPaths::provider_auth_dir()` (`data_paths.rs:386`). **Consequence:** for agents spawned by current builds, Claude history lands at `~/.agentmux/shared/providers/claude/projects/<cwd-slug>/<sid>.jsonl` — already inside `~/.agentmux/`. (Confirmed: that dir contains real `.jsonl` transcripts for `mopeo-06103`, `maks-06103`, etc.)

### 2.3 AgentMux already keeps a per-agent output store

`agent_session.rs` persists, **per agent definition**, in the channel/version FileStore (`filestore.db`):

```
agent:<definition_id>:current/   → output (raw NDJSON stream) + output.state.json (UI snapshot)
agent:<definition_id>:archive:<unix_ms>/   → archived prior sessions (up to 20)
```

This is the *rendered-output* history (used to restore the pane on mount), **per channel-version** — not account-wide, not provider-canonical, not unified or browsable.

### 2.4 The three real defects

1. **cwd-slug fragmentation.** History is keyed by slugified working directory. Agent cwds vary (`mopeo-06065` vs `mopeo-06103`) and **portable builds run from different Desktop paths**, so one logical agent splinters into many history buckets (observed: `C--Users-area54-Desktop-agentmux-0-43-0-…-portable`, `…0-43-1-…`, etc.).
2. **Orphaned pre-isolation histories.** Older sessions live in **global** `~/.claude/projects/` (e.g. the 1.1 MB Mopeo `4f03e3ed` under `mopeo-06065`), outside `shared/providers/`, invisible to current resume.
3. **No unify / organize / clear.** Heterogeneous per-provider layouts; nothing groups by agent, searches, labels, or deletes.

### 2.5 Root cause of "imported agent shows no conversation"

`import-agents.sh` wires an instance to a session it found in **global `~/.claude/projects/`**, but AgentMux resumes with `CLAUDE_CONFIG_DIR=~/.agentmux/shared/providers/claude`, so `claude --resume <sid>` looks under `…/shared/providers/claude/projects/<slug>/<sid>.jsonl` — which doesn't contain that file. **Location mismatch → empty pane.**

---

## 3. Goals / non-goals

**Goals**
- One **provider-agnostic, agent-anchored** history store inside `~/.agentmux/`, durable across channel/version/build-path churn.
- Cover all current providers via per-provider adapters.
- **Organize**: browse/search/label/pin by agent.
- **Clear**: delete per-session / per-agent / bulk, freeing the provider's native files too; size budget + GC.
- **Restore**: open a stopped/imported agent's full prior conversation read-only, without a live resume.
- Fix resume so imported/old sessions actually replay.

**Non-goals**
- Re-implementing provider transcript formats (we normalize, we don't author).
- Live editing of transcripts.
- Cross-provider transcript translation (store each canonically; render uniformly).

---

## 4. Proposed architecture

### 4.1 Store layout (account-wide)

```
~/.agentmux/shared/history/
  index.db                                   # sqlite — organize/search/clear
  agents/<agent_bus_id>/                      # STABLE identity, NOT cwd-slug
    <provider>/<session_id>/
      transcript.jsonl                        # normalized canonical copy
      meta.json                               # provider, cwd, model, counts, ts, labels[], pinned, byte_size
      raw/                                     # optional: byte-for-byte native copy for fidelity/debug
```

Keyed on the agent's **stable identity** (`agent_bus_id` / `definition_id`) so build-path or cwd changes never splinter it. Account-wide (mirrors the `shared/providers` precedent that fixed the auth "validate-spin" churn).

### 4.2 `HistoryAdapter` trait (per provider)

Extend the existing `history/claude_adapter.rs` pattern into a registry:

```rust
trait HistoryAdapter {
    fn provider(&self) -> &'static str;
    /// Find the native session file for (session_id, cwd) given the provider home.
    fn locate(&self, session_id: &str, cwd: &Path, provider_home: &Path) -> Option<PathBuf>;
    /// Normalize a native file into transcript.jsonl + Meta.
    fn normalize(&self, native: &Path) -> Result<(NormalizedTranscript, Meta)>;
    /// Remove the provider's native file(s) for a session (used by clear).
    fn delete(&self, session_id: &str, cwd: &Path, provider_home: &Path) -> Result<()>;
    /// Rehydrate a canonical transcript back into the provider home where --resume expects it.
    fn rehydrate(&self, transcript: &Path, session_id: &str, cwd: &Path, provider_home: &Path) -> Result<()>;
}
```

Impls: `claude` (`projects/<slug>/<sid>.jsonl`), `codex` (`sessions/<y>/<m>/<d>/rollout-*<id>.jsonl`), `gemini` (`history/<id>/`), `kimi`. ACP providers (`copilot`/`pi`/`openclaw`) expose history through the protocol, so their "adapter" captures from the **live stream** (§4.3) rather than a file.

### 4.3 Capture (two sources)

1. **Live tee** — AgentMux already streams provider NDJSON into `agent_session.rs` zones. On turn-end, promote the per-agent `output` into the canonical `transcript.jsonl` (append, dedup by record id). Covers ACP providers that have no file.
2. **Reconcile** — after each turn, the adapter copies the provider's just-written native session file into the store. Catches anything the tee missed and pulls **pre-existing / orphaned** histories on first sight (scan global `~/.claude/projects` + `shared/providers/*` once, attribute to agents by cwd-slug → identity map).

### 4.4 Index + organize

`index.db` rows: `(agent_bus_id, provider, session_id, cwd, model, started_at, ended_at, msg_count, byte_size, preview, labels, pinned)`.

**History pane** (frontend): grouped by agent, searchable (preview/labels), pin/label, and **Open** → loads `transcript.jsonl` into a **read-only document view** (reuse the agent-pane renderer). This solves "imported/stopped agent shows no conversation" without a live resume.

### 4.5 Clear / GC

- Delete per-session / per-agent / bulk; "older than N days" / "over X MB".
- Deletion removes the canonical copy **and** the provider's native file (`adapter.delete`) so it frees space.
- Size budget per `index.db` with **LRU eviction of unpinned** sessions.

### 4.6 Resume correctness (the bug fix)

- Make the agent **cwd deterministic per identity** so the provider slug is stable across builds.
- On resume: if `transcript.jsonl` exists but the provider's isolated-dir copy is missing, **`adapter.rehydrate`** it into `$PROVIDER_HOME/<expected-path>` before spawning `--resume`. Minimal change that makes imported/old sessions replay.

---

## 5. Phasing

| Phase | Scope | Outcome |
|-------|-------|---------|
| **P0** ✅ | Fix `import-agents.sh`: search `shared/providers/claude` + global, and **rehydrate** the chosen session into `$CLAUDE_CONFIG_DIR/projects/<slug>/` where resume reads (`rehydrate_claude_session`) | **DONE** — imported agents' sessions replay on resume (verified: Mopeo `4f03e3ed`, 1.1 MB, copied into the isolated home) |
| **P1** | History store + Claude/Codex/Gemini adapters + turn-end capture + reconcile scan | Real usage durably captured & unified inside `~/.agentmux/` |
| **P2** | `index.db` + History pane (browse/search/label/pin/restore/clear) | Organize + clear + read-only restore |
| **P3** | ACP providers (copilot/pi/openclaw), size budget + GC, cross-machine export | Full coverage + housekeeping |

---

## 6. Open questions

1. **Scope of canonical store:** account-wide `shared/history/` (survives channel/version churn — recommended) vs per-channel. *Recommendation: account-wide.*
2. **Identity key:** `agent_bus_id` vs `definition_id` as the stable anchor (need the one that survives clone/template-promotion — see `agent_session.rs` migrations).
3. **Raw fidelity:** keep `raw/` byte-for-byte native copies (storage cost: Claude alone is ~540 MB locally today) or normalized-only? *Lean: normalized + optional raw behind a setting.*
4. **Capture trigger:** turn-end hook vs filesystem watcher on provider homes. *Lean: turn-end + periodic reconcile.*

---

## 7. Evidence appendix (for implementers)

- Provider registry + resume flags + `auth_dir_name` / `auth_config_dir_env_var`: `providers.rs:98–360`.
- Env assembly + auth-dir injection: `app_api.rs:~1500–1560`; `data_paths.rs:386`.
- Session-id capture from stdout + hydration + resume: `subprocess.rs:322–336, 368–378, 551–656, 825–843`.
- Per-agent output zones + archive + migrations: `agent_session.rs:120, 131, 189, 271, 453, 714`.
- Existing read-only Claude discovery (extend this): `history/claude_adapter.rs`.
- Observed fragmentation: `~/.agentmux/shared/providers/claude/projects/` contains per-build-path buckets (`…Desktop-agentmux-0-43-0-…-portable`, etc.).
- Observed orphan: `~/.claude/projects/C--Users-area54--agentmux-agents-mopeo-06065/4f03e3ed-….jsonl` (1.1 MB) not present under `shared/providers/claude`.
