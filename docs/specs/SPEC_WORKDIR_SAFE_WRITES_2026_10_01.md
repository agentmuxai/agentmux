# SPEC: One safe way to write into an agent's workspace

**Status:** implemented on `clamk/workdir-safe-write` (stacked on #4131).
**Date:** 2026-10-01
**Verified against:** `clamk/lc3-dedupe` @ `ad9cbc40e` (main plus #4131).
**Related:**
- `SPEC_LAUNCH_CONTEXT_WORKSPACE_RULE_AND_STARTUP_FILES_2026_09_30.md` (LC3,
  #4131): its review found seven escape and symlink bugs, one write path at
  a time.
- `RETRO_LAUNCH_CONTEXT_REVIEW_CHURN_2026_10_01.md` (Clamk's workspace), §4
  proposal 1.

## 0. The ask

From the owner, after the retro: "lets write a spec to file and implement
it", meaning the single safe-write helper, so that no write into an agent's
workspace can land outside it or half-write a file, and new code can't
quietly add one that does.

## 1. The problem

AgentMux writes about a dozen files into an agent's working directory at
every launch. The two launch paths are `writeagentconfig`
(`server/editor_handlers.rs`) and `agent.open`'s `write_agent_config_files`
(`server/app_api/agent_open.rs`). Their helpers are in
`backend/agent_config.rs`. The workspace isn't wholly AgentMux's: it can
hold the user's files, a cloned repo, and symlinks.

An inventory found **19 write sites**, each guarding itself differently:

| Gap | Sites |
|---|---|
| No escape check through a symlinked folder (`verify_no_symlink_escape`) | `agent.open`'s generic file loop (skill files, `.claude/settings.json`), stale-skill cleanup deletes, both manifests, the `.mcp.json` lock, `.mcp.json` on the `agent.open` path |
| A symlinked *file* followed by the write (a dangling one even passes `verify_no_symlink_escape`) | every write except the two LC3 added a check to |
| Not atomic (a crash or full disk leaves a truncated file) | everything except `.mcp.json` and its manifest |

Each new write inherits whatever its neighbour does. LC3's review found
the gaps one at a time over 14 commits.

## 2. Design

### 2.1 `backend/workdir_fs.rs`

```rust
/// An agent's working directory, resolved once.
pub struct Workdir { base: PathBuf, canonical: PathBuf }

impl Workdir {
    /// `base` must exist (callers create it first).
    pub fn open(base: &Path) -> io::Result<Workdir>;
    /// `rel` inside the workdir: lexically inside (`safe_join_within_base`),
    /// no ancestor linking out (`verify_no_symlink_escape`), and the leaf
    /// itself not a symlink, dangling or not.
    pub fn resolve(&self, rel: &str) -> io::Result<PathBuf>;
    /// Replace `rel` atomically: temp file in the same folder (`create_new`),
    /// write, `sync_all`, rename over it. Parent folders are created inside
    /// the workdir. `owner_only` sets mode 0600 on Unix.
    pub fn write(&self, rel: &str, bytes: &[u8], owner_only: bool) -> io::Result<()>;
    /// Create `rel` only if absent; `Ok(false)` when it already exists.
    pub fn create_new(&self, rel: &str, bytes: &[u8]) -> io::Result<bool>;
    /// Append to an existing `rel` (O_APPEND, never a rewrite).
    pub fn append(&self, rel: &str, bytes: &[u8]) -> io::Result<()>;
    /// Remove `rel` (a symlink leaf is removed as a link, never its target).
    pub fn remove_file(&self, rel: &str) -> io::Result<()>;
    /// Remove an empty folder `rel`.
    pub fn remove_dir(&self, rel: &str) -> io::Result<()>;
    /// Open (creating if needed) a lock file `rel` for read/write, untruncated.
    pub fn open_lock_file(&self, rel: &str) -> io::Result<File>;
}
```

- **Refusals** are `io::ErrorKind::PermissionDenied` with a message naming
  the path and the reason ("links outside the workspace", "is a symlink").
  The callers keep today's handling: a hard error where they error, a
  logged warning where they're best-effort.
- **Atomic replace on Windows:** `rename` over a file another process holds
  open without delete-sharing can fail. Retry a few times briefly, then
  return the error. Never fall back to a non-atomic write.
- **Rename replaces a symlink entry rather than following it.** So even if
  a link appears between the check and the write, the write stays inside
  the workspace.

### 2.2 Migration: every one of the 19 sites goes through it

| Site | Becomes |
|---|---|
| Generic file loop, both launch paths (skill files, `.claude/settings.json`, …) | `write` |
| `CLAUDE.md` (owned or new), `AGENTS.md`/`GEMINI.md`/`QWEN.md`/`.pi/APPEND_SYSTEM.md` | `write` (the ownership decision stays in the caller) |
| `.claude/AGENTMUX_MEMORY.md`, the skill-file manifest | `write` |
| `.mcp.json`, `.claude/.agentmux-managed-mcp-servers.json` | `write(…, owner_only: true)`; `write_owner_only_atomically` is folded in |
| `.claude/.agentmux-mcp-json.lock` | `open_lock_file`; a refusal writes `.mcp.json` unlocked (still atomic) instead of failing the launch |
| Ownership marker, legacy backup | `create_new` |
| The `@import` line appended to a foreign `CLAUDE.md` | `append` |
| Stale-skill cleanup, marker rollback | `remove_file` / `remove_dir` |

**Out of scope:** creating the workdir itself (`allocate_agent_workdir`,
`create_dir_all` of the base, the controller's `apply_working_dir`); it
isn't a write *into* the workdir. Also out: writes to AgentMux's own data
and config dirs, and user-initiated editor saves.

### 2.3 A gate so it stays that way

`scripts/check-workdir-writes.mjs`, in `ci-pr.yml`'s "doc status + grep
gates" job next to `check-time-helpers.mjs`. It scans
`backend/agent_config.rs` and `server/app_api/agent_open.rs` line by line,
and in `server/editor_handlers.rs` only the `writeagentconfig` handler (the
rest of that file is the editor pane's own saves). It skips `#[cfg(test)]`
modules. It fails on a raw `fs::write`, `File::create`,
`OpenOptions`, `fs::rename`, `fs::remove_file`, `fs::remove_dir`,
`fs::copy` or `create_dir_all`. The escape hatch is a same-line
`// workdir-fs: <reason>` comment, for the base-creation lines above.

### 2.4 Not changed here

`.claude/settings.json` always replaces a user's own file at that path.
That's an ownership policy question, not a safety one. Noted for a
follow-up.

## 3. Tests

`workdir_fs` unit tests:
- `resolve` refuses an absolute path, `..`, a symlinked leaf (live and
  dangling), and a symlinked parent that links outside;
- `write` creates parents, replaces atomically, leaves no temp file behind
  on success or failure, and sets 0600 when asked (Unix);
- `create_new` doesn't overwrite;
- `append` appends;
- `remove_file` removes a link, not its target.

Symlink tests are skipped where the OS won't create a link.

The existing `agent_config`, `editor_handlers` and `agent_open` tests must
pass unchanged, apart from the two LC3 symlink tests, which move to
asserting the helper's refusal. A new test covers `agent.open`'s generic
loop no longer writing through a `.claude` that links outside, and the launch
still succeeding. Run against the code before this change, the gate lists 27
raw writes; after it, none.

## 4. Risk

- **Behaviour change:** writes that were unsafe (through a link, outside
  the workspace) are refused and logged instead of performed. Intended. A
  setup that relied on writing through a symlinked `CLAUDE.md` stops being
  updated.
- **Atomic replace changes a file's identity** (a new inode). Anything
  holding the old file open keeps the old contents, which is already true
  of `.mcp.json` today.
