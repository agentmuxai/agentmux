# SPEC: Shared filesystem-watcher framework — audit + design

**Date:** 2026-08-07
**Status:** implemented — framework PR #2455 (backend/fs_watch/ FsWatchPool), consumers migrated in #2456 (config watcher) and #2462 (editor + media); verified in code 2026-08-10.
**Trigger:** Follow-up to `docs/specs/SPEC_NATIVE_MEMORY_DURABLE_SYNC_2026_08_07.md`,
which deferred live capture of Claude's autonomous memory writes as a
non-goal specifically because it would need a filesystem watcher. User
asked: do we already have watchers, and should there be one shared,
robust framework (with error handling + recovery) everything lives under?

---

## 0. Answers

**Do we already have watchers?** Yes — four independent, hand-rolled
implementations, all on the `notify` crate (already a dependency,
`agentmux-srv/Cargo.toml:37`, v7). See §1.

**Should there be a shared framework?** Yes, for two independent reasons:

1. **Three of the four already duplicate the same ~80 lines of
   plumbing**, and say so in their own doc comments (§2) — this isn't a
   hypothetical future benefit, it's dedup of code that exists today.
2. **None of the four have any error recovery** — every one logs a warning
   and silently gives up if the initial watch fails, and none re-check or
   re-subscribe if the OS-level watch dies later. This is a real gap, not
   just style — it's the specific thing blocking the native-memory spec's
   deferred follow-up, and it's invisible today because every existing
   consumer treats live-update as "nice to have," so a silently-dead watcher
   just looks like "nothing changed" instead of an error. §3.

---

## 1. Existing watchers, audited

| File | Watches | Scope | Debounce | Error on create | Recovery if watch later dies |
|---|---|---|---|---|---|
| `agentmux-srv/src/backend/config_watcher_fs.rs` | `settings.json`'s directory | One fixed path, global | 300ms, drain-loop | log + `None`, feature silently off | none |
| `agentmux-srv/src/backend/editor_file_watcher.rs` | Arbitrary open-editor-tab files | Dynamic set, per-block refcounted | 300ms, per-path generation counter | log + `None`, feature silently off | none |
| `agentmux-srv/src/backend/media_file_watcher.rs` | Arbitrary Media-pane directories | Dynamic set, per-block refcounted, extension-filtered | 300ms, per-path generation counter | log + `None`, feature silently off | none |
| `agentmux-srv/src/backend/subagent_watcher/` | Claude Code session/subagent dirs | Dynamic set, per-block, full module (parse/scan/jsonl/query submodules) | domain-specific (JSONL line-buffering, 500ms activity-flush) | log + presumably `None`/panic-avoided (not re-checked here) | partial — has its own **reconciliation** (`scan.rs`'s `reconcile_stale_subagents`, event-triggered on confirmed-idle) for stale *application state*, but that's a different problem from "the OS watch itself died" |

All four constructed independently in `agentmux-srv/src/bootstrap.rs`
(lines ~852, 860, 887, 1117) — no shared registry, no shared lifecycle, no
shared health surface.

## 2. Duplication, quantified

`editor_file_watcher.rs`'s own doc comment: *"Generalizes
`config_watcher_fs.rs`'s single-file `notify`-watcher pattern to an
arbitrary, dynamically-changing set of paths."* `media_file_watcher.rs`'s own
doc comment: *"Directory-mode sibling of `editor_file_watcher.rs`'s
single-file watcher."* This is explicit, self-documented copy-adapt
evolution — not speculation. Concretely duplicated across editor+media (near
byte-identical in places):

- `notify::recommended_watcher` construction + the sync-callback → `mpsc`
  channel → async `tokio::spawn` consumer bridge (`new()`, ~35 lines each).
- Per-path debounce via an `AtomicU64` generation counter per watched path,
  pruned on unwatch (`handle_fs_event`, ~25 lines each).
- Refcounted watch/unwatch bookkeeping so a directory is only actually
  under OS watch while ≥1 subscriber cares about it.
- **The correctness-critical decision to watch the parent directory, not the
  file itself**, `RecursiveMode::NonRecursive` — necessary so an
  atomic-write-via-tmp-then-rename save (exactly what
  `native_memory_handlers.rs`'s own `write_file` handler does, and what most
  editors/tools do) is still detected; watching a file's own inode directly
  can silently stop firing after a rename-over-target. Every implementer had
  to independently know this. A shared framework should bake it in once.
- Publishing a WPS event scoped to the subscribing block IDs, never a global
  broadcast (`publish_editor_file_changed`/`publish_media_file_changed` are
  structurally identical).

`config_watcher_fs.rs` predates the refcounted-subscriber pattern (it only
ever watches one fixed path) but duplicates the construction + debounce
shape inline in its own `spawn_settings_watcher`.

## 3. The actual gap: zero error recovery, anywhere

Every one of the four handles watcher-creation failure the same way: log a
`tracing::warn!` and return `None`/skip setup. That's a reasonable *initial*
posture (live-update is additive, not required for the feature to work at
all) — but none of them handle **degradation after a successful start**:

- If the OS-level watch dies mid-session (inotify instance-limit exhaustion,
  a network-drive/WSL mount where inotify is flaky, the watched directory
  itself getting deleted and recreated at a new inode), there is no
  detection, no retry, no re-subscribe, and no signal anywhere (`muxlog
  errors` included) that live-update quietly stopped working. The feature
  just looks like "nothing has changed" — indistinguishable from "the file
  genuinely hasn't changed" from the user's side.
- `notify` itself ships a `PollWatcher` fallback (stat-based, works
  everywhere the native backend doesn't) — none of the four ever reach for
  it.

This is precisely the gap the native-memory durability spec's §2.3 flagged
as a "known residual risk... not proposed here": closing it requires exactly
this missing recovery layer, and building it once, shared, is a better
trade than adding a bespoke retry loop to a fifth watcher.

## 4. Design

New module `agentmux-srv/src/backend/fs_watch/` (mirrors `subagent_watcher/`
being its own directory once it outgrew one file):

```rust
/// One shared notify backend + subscription registry. Domain modules
/// (editor, media, config, subagent, and the new native-memory one) each
/// hold an `Arc<FsWatchPool>` and call subscribe/unsubscribe; they own
/// their own debounce policy and event payload/publish shape, since those
/// are genuinely domain-specific (editor wants per-path, media wants
/// per-directory+extension-filter, subagent wants JSONL-aware buffering).
pub struct FsWatchPool { /* ... */ }

pub struct Subscription {
    pub id: SubscriptionId,
    pub path: PathBuf,
    pub recursive: bool,
}

impl FsWatchPool {
    pub fn new() -> Arc<Self>;

    /// Start watching `path` on behalf of `owner` (refcounted — matches the
    /// existing editor/media pattern, generalized). Always watches the
    /// parent directory non-recursively when `path` is a file, per §2's
    /// baked-in atomic-rename-safety rule; recursive directory watches are
    /// opt-in via `recursive: true`.
    pub fn subscribe(&self, path: &Path, owner: SubscriberId, recursive: bool) -> Subscription;
    pub fn unsubscribe(&self, sub: &Subscription);

    /// Raw change stream — domain modules apply their own debounce/filter on
    /// top rather than the pool imposing one policy for everyone (editor's
    /// 300ms-per-path and subagent's 500ms-activity-flush are both
    /// legitimate, different needs).
    pub fn events(&self) -> broadcast::Receiver<FsWatchEvent>;

    /// Current health snapshot — the new capability none of the four have
    /// today. Exposed for `muxlog`/diagnostics, not just internal use.
    pub fn health(&self) -> FsWatchHealth;
}

pub struct FsWatchHealth {
    pub backend: WatchBackend,        // Native | Polling (which one is actually active)
    pub active_watches: usize,
    pub degraded_paths: Vec<(PathBuf, String)>, // path -> last error, still being retried
}
```

**Recovery policy (the part that's actually new, not just dedup):**

1. **On initial `watch()` failure**: retry with exponential backoff
   (bounded — e.g. 3 attempts over ~5s) before giving up and logging at
   `warn`, rather than failing on the first attempt. Transient failures
   (directory not yet created, brief permission hiccup during another
   process's own atomic write) shouldn't need a full restart to recover
   from.
2. **Degraded-native fallback**: if the native backend
   (`RecommendedWatcher`) fails to construct at all, fall back to `notify`'s
   own `PollWatcher` (interval configurable, default a few seconds) instead
   of disabling live-update outright — worse latency, still correct.
3. **Silent-death detection**: no perfect way to know an inotify watch died
   without OS support notify doesn't expose — so this is necessarily
   heuristic. Pragmatic version: a low-frequency (e.g. every 30s) background
   sweep that re-verifies every currently-"active" watched path still
   resolves (`Path::exists()` / re-stat) and, for paths that do, confirms
   the underlying watch descriptor is still registered with the OS backend
   (`notify` exposes this on some platforms; where it doesn't, re-issuing
   `watch()` on an already-watched path is a safe, cheap no-op per the
   crate's own docs) — self-healing without needing a true "is this dead"
   signal.
4. **Observable, not just logged**: `FsWatchHealth` surfaces `degraded_paths`
   so a future diagnostics surface (or just `muxlog errors`, which already
   greps host+sidecar logs) can show "this has been failing for N minutes,"
   not just a single warn line that scrolls out of view.

## 5. Migration path

Not a big-bang rewrite. Lowest-risk order:

1. **Build `fs_watch::FsWatchPool`** with the recovery policy above, no
   consumers yet — pure addition, zero behavior change to anything live.
2. **Migrate `config_watcher_fs.rs`** first — smallest, single fixed path,
   easiest to verify byte-for-byte behavior parity.
3. **Migrate `editor_file_watcher.rs` and `media_file_watcher.rs`** —
   they already share the same shape, so this is mostly deleting their
   duplicated plumbing and keeping their genuinely-distinct debounce/publish
   logic as thin wrappers over `FsWatchPool`.
4. **`subagent_watcher`** last, and possibly not fully — it's the most
   mature and most domain-coupled (JSONL parsing, dispatch state); only
   worth moving its raw notify-subscription layer onto the shared pool, not
   its scan/reconciliation logic, which is legitimately its own thing.
5. **New consumer: native memory.** Once `FsWatchPool` exists, the
   native-memory durability spec's deferred §2.3 gap (capturing Claude's
   autonomous writes between Stash tab opens) becomes a straightforward
   addition: subscribe to each agent's resolved memory directory while its
   Stash Memory tab is open (or, if wanted later, for the lifetime of a live
   agent process), and on a change event, run the same upsert-into-mirror
   logic §2.2 of that spec already added to the read path — just triggered
   by a push instead of only a pull. This closes that spec's one accepted
   gap essentially for free once the shared framework exists, rather than
   needing its own bespoke watcher.

## 6. Non-goals

- Frontend/TypeScript file watching (Vite's own dev-server watcher is
  separate infrastructure, out of scope — nothing in the app's own runtime
  code watches files from the frontend process).
- Forcing `subagent_watcher`'s domain logic (JSONL parsing, dispatch state
  machine, reconciliation) onto the shared pool — only its raw
  subscribe/unsubscribe plumbing is a migration candidate.
- A UI-visible "watcher health" panel — `FsWatchHealth` is designed to be
  cheap to add one later, not proposing one now.

## 7. Test plan

- Unit: retry-with-backoff on a forced `watch()` failure (inject via a
  path that doesn't exist yet, then create it mid-retry) succeeds within the
  bounded attempt window.
- Unit: `PollWatcher` fallback actually detects a change when the native
  backend is unavailable (test harness forces the fallback path rather than
  relying on CI's sandbox lacking inotify, which is a real but unreliable
  way this already gets partially exercised today per `editor_file_watcher.rs`'s
  own test comment: "CI sandboxes may not support inotify").
- Unit: refcounted subscribe/unsubscribe parity with the existing
  editor/media tests (port those tests onto the new pool directly — they're
  already written against exactly this behavior).
- Integration: simulate an atomic rename-over-target save and confirm a
  directory-level subscription still fires (regression guard for §2's
  baked-in rule).
- Manual: exhaust something to force a degraded state (e.g. temporarily
  chmod a watched directory unreadable), confirm `FsWatchHealth.degraded_paths`
  reflects it and self-heals once permissions are restored, without a
  process restart.
