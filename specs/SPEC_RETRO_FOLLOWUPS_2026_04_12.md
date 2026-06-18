# Spec: Retro Follow-ups — 2026-04-12

**Status:** Draft — investigation complete, implementation pending approval
**Origin:** Open questions from two retros written on 2026-04-12:
- `docs/retro/2026-04-12-ultra-long-sessions.md` §7
- `docs/retro/2026-04-12-portable-size-audit.md` §6

This spec investigates each open question, diagnoses the root cause, and
proposes a concrete implementation fix with exact file paths and line numbers.
Follow-ups that are monitoring-only or explicitly "not now" are noted at the
end but not detailed.

---

## Follow-ups covered in this spec

| # | Source | Question                                               | Disposition |
|---|--------|--------------------------------------------------------|-------------|
| 1 | ULS §7.1 | bump-cli doesn't sync `package-lock.json`            | **Fix** — config change + npm lockfile fallback |
| 2 | ULS §7.2 | Nested git clone in `~/.agentmux/agents/agentx/`     | **Fix** — runtime guard + logs |
| 3 | Size §6.1 | Keep / rename / delete pre-ANGLE CEF ZIP            | **Cleanup** — delete with tombstone note |
| 4 | Size §6.2 | "Why did `wsh` stop being duplicated?"              | **Correction** — audit was wrong; the dup is a runtime side effect, not a packaging bug. Separate fix proposed. |
| 5 | Size §6.3 | Track per-version uncompressed size in `VERSION_HISTORY.md` | **Fix** — script + bump-cli hook |
| 6 | Size §6.4 | Packaging script silent Compress-Archive failure     | **Already fixed** in commit `3390e29` (in-tree). No further action. |

Deferred / not addressed here:
- ULS §7.3 WER dump collection — passive monitoring, no action until dumps appear.
- ULS §7.4 Rate-limit telemetry — explicit "not now" in the retro.

---

## Follow-up #1 — bump-cli package-lock.json drift

### Observation

On every `bump patch --commit` this session, the tool printed:

```
npm: npm version completed with warning: Unknown error
cargo: Updated
```

and left `package-lock.json` at the previous version. The lockfile had to be
re-synced manually with `npm install --package-lock-only` before pushing. PR
#341 review flagged this as a real regression because the committed state had
`package.json = 0.33.99` but `package-lock.json = 0.33.98`.

### Root cause

`.bump.json` configures npm lockfile handling as:

```json
"lockfiles": [
  {
    "type": "npm",
    "strategy": "npm-version",
    "command": "npm install --package-lock-only",
    "allowFailure": true
  },
  ...
]
```

The `"strategy": "npm-version"` tells bump-cli to try `npm version <new>` as
the primary mechanism. That command writes both `package.json` and
`package-lock.json` on success, but fails with "Unknown error" in a non-git
working tree (bump-cli temporarily detaches git state during the bump).
Because `allowFailure: true`, bump-cli ignores the failure and moves on —
**without falling back to the `command` field**. The result is that
`package-lock.json` is never updated.

This is a bug in bump-cli's lockfile strategy handling, but we can work around
it in our config without waiting for an upstream fix.

### Fix

Change `.bump.json` to skip the `npm-version` strategy entirely and always run
`npm install --package-lock-only` directly:

```json
"lockfiles": [
  {
    "type": "npm",
    "strategy": "command",
    "command": "npm install --package-lock-only --ignore-scripts",
    "allowFailure": false
  },
  {
    "type": "cargo",
    "command": "cargo generate-lockfile"
  }
]
```

Changes:
- `strategy: "command"` — use the `command` field as the primary mechanism, no `npm version` attempt.
- `--ignore-scripts` — prevents any lifecycle hooks from running during the lockfile refresh (faster, safer inside bump-cli's locked state).
- `allowFailure: false` — if the lockfile can't be written, the bump should fail loudly. It's better to abort the version bump than to create a committed inconsistency.

Alternative if the above doesn't work (bump-cli may not honor `strategy: "command"`):

- Drop the `"type": "npm"` entry entirely and add a post-bump hook in `scripts/bump-post.sh` that runs `npm install --package-lock-only` and stages the result. Then wire it via `.bump.json` `"hooks": { "post": "scripts/bump-post.sh" }`.

### Verification

After the fix, run:

```bash
bump patch -m "test post-bump lockfile sync" --commit
git show HEAD --stat | grep package-lock
```

Expected: `package-lock.json` appears in the commit and its version line matches `package.json`.

### Affected files

- `.bump.json` — config change only, ~6 lines.

---

## Follow-up #2 — Nested git clone in agent workspaces

### Observation

During earlier debugging, a 3.5 GB nested clone of the AgentMux repo was
discovered under `~/.agentmux/agents/agentx/agentmux/`. It was thrashing
Windows I/O and confusing agents inside the pane into reading pre-SolidJS
React code. Deleted manually, but nothing in the current codebase prevents
recurrence.

### Root cause

In `agentmux-srv/src/server/app_api.rs:142-149`, the default agent working
directory is:

```rust
let work_dir = if agent.working_directory.is_empty() {
    format!("~/.agentmux/agents/{}", agent_slug)
} else {
    agent.working_directory.clone()
};
```

This is fine as a *cwd* for the spawned CLI, but if the agent's first action
is `git clone <url> .` or `cp -r $PROJECT_ROOT .`, it populates that directory
with a full repo that agentmux has no knowledge of. Nothing in the flow
prevents it, and nothing detects it after the fact.

Worse: multiple agents sharing the same parent directory
(`~/.agentmux/agents/`) can accumulate duplicate clones that consume disk
space linearly in the number of agents.

### Fix

Two-layer defense: **detect + warn on startup**, plus **document + prune**.

**Layer 1: runtime warning on agent controller spawn.**

In `agentmux-srv/src/backend/blockcontroller/persistent.rs` (the persistent
controller's `spawn_process` function), after the `current_dir` is set but
before the `cmd.spawn()` call, add:

```rust
// Warn loudly if the agent working directory contains a nested git repo.
// This happens when an agent clones a repo into its own cwd, which can
// consume gigabytes and shadow the real project's state from agents that
// scan the working tree. Detection is cheap — one stat call — and the
// warning gives the user a clear action item without blocking the spawn.
if let Some(cwd) = std::path::Path::new(&expanded_dir).canonicalize().ok() {
    let nested_git = cwd.join(".git");
    let is_agent_data_dir = expanded_dir.contains("/.agentmux/agents/");
    if is_agent_data_dir && nested_git.exists() {
        let size = directory_size_mb(&cwd).unwrap_or(0);
        tracing::warn!(
            block_id = %self.block_id,
            cwd = %cwd.display(),
            size_mb = size,
            "agent working dir contains a nested git repo — this is \
             usually unintended. Remove with: rm -rf {}/.git", cwd.display()
        );
    }
}
```

Also add a small helper:

```rust
fn directory_size_mb(path: &std::path::Path) -> Option<u64> {
    walkdir::WalkDir::new(path)
        .max_depth(4)  // bounded — we only need order-of-magnitude
        .into_iter()
        .filter_map(|e| e.ok())
        .filter_map(|e| e.metadata().ok())
        .map(|m| m.len())
        .try_fold(0u64, |acc, n| acc.checked_add(n))
        .map(|bytes| bytes / 1024 / 1024)
}
```

(Add `walkdir = "2"` to `agentmux-srv/Cargo.toml` if not already present —
it's a tiny, mature crate. If already available, reuse it.)

**Layer 2: `~/.agentmux/.gitignore` seeded on first launch.**

Add one line to the first-launch bootstrap in `main.rs` (near
`ensure_initial_data`):

```rust
// Drop a .gitignore so any accidental git operations inside the data
// directory don't leak metadata into the user's primary project (or
// into agent workspaces that happen to be inside a git repo).
if let Some(home) = dirs::home_dir() {
    let data_dir = home.join(".agentmux");
    let gitignore = data_dir.join(".gitignore");
    if data_dir.is_dir() && !gitignore.exists() {
        let _ = std::fs::write(&gitignore, "*\n!.gitignore\n");
    }
}
```

**Layer 3: documentation.**

Add a short "Agent workspaces" section to `BUILD.md` or
`docs/AGENT_AUTH_STATE_MACHINES.md` explaining:
- Default working dir is `~/.agentmux/agents/<slug>/`.
- If your agent clones a repo into its cwd, that clone lives in the agent's
  private workspace — not the project tree — and will persist until you
  delete it.
- `rm -rf ~/.agentmux/agents/<slug>` is safe to run while the agent pane is
  closed.

### Why not just block the clone?

We can't — the agent is an external CLI process. Any enforcement has to come
from detection after the fact, plus a loud warning, plus easy cleanup. That's
what layers 1-3 provide.

### Affected files

- `agentmux-srv/src/backend/blockcontroller/persistent.rs` — ~25 lines
- `agentmux-srv/src/main.rs` — ~10 lines
- `agentmux-srv/Cargo.toml` — `walkdir = "2"` (only if not already present)
- `BUILD.md` or a new `docs/agent-workspaces.md` — a short section

---

## Follow-up #3 — Delete the pre-ANGLE CEF ZIP

### Observation

`dist/agentmux-cef-portable.zip` dated 2026-03-29 is the only surviving
snapshot of the pre-ANGLE-DLL portable bundle. It's smaller than every build
after it (147.7 MiB vs ~155 MiB current), which caused the "why did the build
gain 10 MB" confusion at the top of the size audit.

### Fix

Delete it. The file has no legitimate use:
- It's not tagged, not versioned, not referenced by any script.
- It's smaller only because it's missing load-bearing GPU DLLs, so extracting
  and running it would regress rendering.
- Keeping it around is how the "10 MB regression" phantom showed up.

One-line action:

```bash
rm C:/Systems/agentmux/dist/agentmux-cef-portable.zip
```

Optionally replace with a tombstone text file if anyone might look for it
later:

```bash
cat > C:/Systems/agentmux/dist/README.md <<EOF
# dist/ artifacts

This directory holds build outputs from `task cef:package:portable` and
`task build:backend`. Historical ZIPs are NOT kept here — look in git or
GitHub Releases. The only files that should persist in git are:

- Small Tauri-era packaged ZIPs kept as reference points (<20 MB each).

Large CEF portable ZIPs (>100 MB) are .gitignored.
EOF
```

### Affected files

- `dist/agentmux-cef-portable.zip` — deleted
- `dist/README.md` — new, one paragraph
- `.gitignore` — verify `dist/agentmux-cef-*-portable.zip` is listed (it should be; if not, add it)

---

## Follow-up #4 — The `wsh` "dedup" was a false reading

### Correction to the audit

The size audit claimed the 0.33.91 → 0.33.101 delta included a 1.19 MiB
reduction from "wsh binary deduplication." That was wrong. The 0.33.91 ZIP
was clean — I verified:

```
$ pwsh -Command "... OpenRead('agentmux-cef-0.33.91-x64-portable.zip').Entries
                 | Where FullName -like '*wsh*' | Select FullName, Length"
FullName                            Length
--------                            ------
runtime/wsh-0.33.91-windows.x64.exe 1191424
```

Only one copy inside the ZIP. The `runtime/bin/wsh-*.exe` file I found on
disk was **created at runtime** by the CEF host, not by the packager.

### Real root cause

`agentmux-cef/src/sidecar.rs:376-440` has a `deploy_wsh()` helper:

```rust
fn deploy_wsh(app_path: &std::path::Path) {
    let bin_dir = app_path.join("bin");
    std::fs::create_dir_all(&bin_dir).ok();
    let bundled_wsh = find_wsh_source(app_path);
    ...
    let wsh_name = format!("wsh-{}-{}.{}{}", version, goos, goarch, exe_suffix);
    let dest = bin_dir.join(&wsh_name);
    if dest.exists() {
        return; // already deployed
    }
    std::fs::copy(&bundled_wsh, &dest) ...
}
```

Called on every CEF host startup. In a portable layout:

- `app_path` = `runtime/`
- `find_wsh_source` finds `runtime/wsh-0.33.91-windows.x64.exe` (the packaged copy)
- `deploy_wsh` creates `runtime/bin/` and copies it to `runtime/bin/wsh-0.33.91-windows.x64.exe`

So every extracted + launched portable ends up with **two identical 1.19 MiB
files** on disk — a wasted copy that happens *outside* the ZIP.

### Fix

The `deploy_wsh` helper should no-op when the source already lives in the
target app_path at the same version. Edit `agentmux-cef/src/sidecar.rs`
around line 425:

```rust
let dest = bin_dir.join(&wsh_name);

// If the bundled wsh is already at app_path/<versioned-name>, there's no
// reason to copy it into app_path/bin/. The spawner will find it in place
// via find_wsh_source() on every lookup. Only copy if the source is in a
// different location (e.g. dev mode with dist/bin/wsh-*.exe outside the
// app_path).
if let Ok(src_canon) = bundled_wsh.canonicalize() {
    if src_canon.parent() == Some(app_path) {
        tracing::debug!(
            source = %bundled_wsh.display(),
            "wsh already lives in app_path; skipping deploy"
        );
        return;
    }
}

if dest.exists() {
    return; // already deployed
}
```

Also audit any code that looks wsh up at runtime: `find_wsh_source` already
scans `app_path` for any `wsh-*.exe` (lines 457-466), so it will find the
packaged file with no additional changes. The sidecar spawn code that
actually launches wsh needs to accept either path (`app_path/wsh-*.exe` OR
`app_path/bin/wsh-*.exe`). Verify before merging — grep for `.join("bin")`
in `agentmux-cef/src/` and confirm the spawn logic handles both.

### Expected impact

- Saves 1.19 MiB of disk per extracted portable folder.
- Eliminates a redundant file system write on every CEF host startup.
- No effect on the ZIP itself — the ZIP was already clean.

### Affected files

- `agentmux-cef/src/sidecar.rs` — ~15 lines around `deploy_wsh`
- Also: a one-paragraph note in the audit retro correcting the earlier "wsh dedup" claim.

---

## Follow-up #5 — Track per-version uncompressed size

### Goal

Make "did this release get bigger?" answerable without re-extracting ZIPs.
Single-line size entry per release, written at package time.

### Fix

Option A (cheapest, preferred): append to `VERSION_HISTORY.md` from the
packaging script.

Add to the end of `scripts/package-cef-portable.sh` after the success line:

```bash
# Append a compact size row to VERSION_HISTORY.md (if present).
# Format: | version | date | compressed MiB | uncompressed MiB | note |
if [ -f ../VERSION_HISTORY.md ] || [ -f VERSION_HISTORY.md ]; then
    HIST_FILE="VERSION_HISTORY.md"
    [ -f ../VERSION_HISTORY.md ] && HIST_FILE="../VERSION_HISTORY.md"

    DIR_BYTES=$(find "$PORTABLE" -type f -printf '%s\n' 2>/dev/null | awk '{s+=$1} END {print s+0}')
    ZIP_BYTES=$(stat -c '%s' "$ZIP_NAME" 2>/dev/null || echo 0)
    DIR_MIB=$(awk "BEGIN {printf \"%.1f\", $DIR_BYTES/1024/1024}")
    ZIP_MIB=$(awk "BEGIN {printf \"%.1f\", $ZIP_BYTES/1024/1024}")

    # Insert above the "## Version History" header
    awk -v ver="$VERSION" -v date="$(date +%Y-%m-%d)" -v zip="$ZIP_MIB" -v dir="$DIR_MIB" '
        /^## Sizes / { in_sizes=1 }
        /^##/ && !/^## Sizes / { if (in_sizes) { print "| " ver " | " date " | " zip " MiB | " dir " MiB | |"; in_sizes=0 } }
        { print }
    ' "$HIST_FILE" > "$HIST_FILE.tmp" && mv "$HIST_FILE.tmp" "$HIST_FILE"
fi
```

And add a section stub to `VERSION_HISTORY.md`:

```markdown
## Sizes (CEF portable, Windows x64)

Auto-appended by `scripts/package-cef-portable.sh`. Oldest last.

| Version | Date | ZIP (compressed) | Folder (uncompressed) | Note |
|---------|------|------------------|-----------------------|------|
| 0.33.102 | 2026-04-12 | 155.5 MiB | 320.0 MiB | post-ultra-long-sessions |
| 0.33.91  | 2026-04-12 | 151.8 MiB | 320.8 MiB | pre-ultra-long-sessions |
| (pre-ANGLE) | 2026-03-29 | 147.7 MiB | 309.8 MiB | no libEGL/libGLESv2 |
```

Option B (more ambitious): a `scripts/size-report.sh` that runs over *every*
zip in `~/Desktop/agentmux-cef-*-portable.zip`, emits the same table, and is
wired into `task cef:package:portable` as a post-step. Deferred unless we
start doing release-over-release comparisons regularly.

### Verification

Run `task cef:package:portable`, then `tail -n 3 VERSION_HISTORY.md`. A new
row for the current version should appear.

### Affected files

- `scripts/package-cef-portable.sh` — ~20 lines appended
- `VERSION_HISTORY.md` — new "## Sizes" section

**Also:** fix the stale bump-cli history integration. The tool logs _"Could
not update VERSION_HISTORY.md (file not found or pattern not matched)"_ on
every run because the config's `template` format (`| {{version}}-fork | v0.12.0 | ...`)
doesn't match the file's actual layout. Either:
- Drop the history section from `.bump.json` entirely (cleanest — the file
  hasn't been touched by bump since March), or
- Rewrite `VERSION_HISTORY.md`'s top to use the table format the template
  expects.

Recommend the first. Keep history updates manual or hook it into the
packaging script per option A above.

---

## Follow-up #6 — Packaging script silent ZIP failure (already fixed)

Fixed in commit `3390e29` (local, unpushed at time of writing). The
replacement logic tries `pwsh` first, falls back to Windows PowerShell 5,
then to `tar -a -cf`, and exits non-zero if all three fail. Verified working
on the 0.33.102 build. No further action in this spec.

---

## Implementation order

When approved, execute in this order — each item is independent but some
share a branch to reduce review noise:

1. **Quick wins** (single PR, ~30 min total):
   - Follow-up #1 — `.bump.json` fix
   - Follow-up #3 — delete pre-ANGLE ZIP + `dist/README.md`
   - Follow-up #5 — VERSION_HISTORY sizes (script + stub)
   - Strip stale bump history config

2. **Runtime behavior** (separate PR, ~1 hour):
   - Follow-up #2 — nested git clone detection
   - Follow-up #4 — `deploy_wsh` no-op when source in place
   - Correction note in the size audit doc

Both branches should bump patch, run `cargo check -p agentmux-srv` and
`bump verify`, and go through reagent review. The runtime PR should include
a manual test plan step for the wsh `deploy_wsh` fix (make sure terminals
still open after the skip).

---

## Non-goals

- WER dump collection triggers (ULS §7.3) — nothing to do until a dump
  shows up.
- Runaway-output rate limiting (ULS §7.4) — explicitly deferred; the meta
  slot in `session:*` is available if we later want to wire bytes/sec.
- Whole rewrite of `.bump.json` lockfile handling (upstream bump-cli bug).
- Any refactor of the portable layout — the launcher / `runtime/` split is
  working well and is load-bearing for multi-version coexistence.
