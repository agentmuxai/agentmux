# settings.json lists every setting with its default commented out, kept 1-to-1 by a guard, safe to copy and paste

**Status:** active. Approved 2026-11-01 by the operator ("yes, green light on settings spec", relayed by AgentY): decisions 1-6 as recommended. Built in the order of §8, one PR each, each updating this line.
**Date:** 2026-11-01.
**Requested by:** the operator, through AgentY (task #16467, build #16468). Their words (2026-11-01): "in that case, should the settings.json should have all the settings with defaults commented out. currently it appears to be a subset. we also want to add a guard that gets 1-to-1. This makes it easy for users save and restore their settings. for now they can just copy/paste into the file. lets also think through edge cases and add guards".
**Author:** Loap@narko.
**Checked against:** `origin/main` at `e4f374c61` (v0.60.10). Numbers below are from that commit.
**Related:** `frontend/app/view/settings/settings-defaults.test.ts` (the existing check that defaults agree), `docs/analysis/settings-json-divergence.md` (why old files look different), Agent1's #16466 (opens settings.json in AgentMux's Editor), Opaz (the docs page).

---

## 1. Summary

`settings.json` should be the one place a person can read every setting, see what it defaults to, and save or restore their whole configuration by copying the file. Today it is a subset, nothing ties it to the code, and a few ordinary actions (a half-finished edit, a file with a byte-order mark, two writers) can destroy the user's file. This spec does four things:

1. **One source of truth** for every setting: `schema/settings.json`, which the defaults test already calls the source of truth, gains the few fields it lacks. The template is generated from it.
2. **A template that shows every setting**, grouped, each with its description and its default commented out. A value the user sets replaces its commented line in place.
3. **A guard that makes the code, the schema, the template and the docs 1-to-1**, with a baseline that can only shrink.
4. **A safe write path**: one writer, edits only the lines it means to change, never overwrites a file it could not read, atomic, and a clear error with a "Fix in editor" link for a broken file.

§7 lists 29 edge cases, each with its guard and its test. §8 is the PR order; the first PR (§8, step 1) stops a data-loss bug that exists today and can ship ahead of the rest.

## 2. What exists today (verified on `origin/main`)

| Piece | Where | Fact |
|---|---|---|
| Template | `settings-template.jsonc` (repo root), embedded in srv by `include_str!` (`wconfig/mod.rs:31`) | 123 commented keys, grouped by `// -- Section --` headers, hand-written. |
| Schema | `schema/settings.json` | 187 property entries, 102 with a `default`, 134 with a `description`. 64 of them are not in the template; some are wildcard families (`app:*`, `term:*`, `window:*`, `notify:*`, `telemetry:*`, `conn:*`, `widget:*`). |
| Typed settings | `crates/srv/src/backend/wconfig/types.rs` (`SettingsType`) | About 140 renamed keys, plus `#[serde(flatten)] extra` for everything else. srv and the frontend read many keys that are not struct fields. |
| Defaults check | `frontend/app/view/settings/settings-defaults.test.ts` | Cross-checks the *default value* of each key across the schema, template, Settings pane, frontend and backend, with `KNOWN_MISMATCHES` and `SINGLE_SOURCE` lists that may only shrink. It does **not** check that every key exists everywhere. |
| Writers | `merge_into_template`, **two copies**: `crates/cef/src/commands/platform.rs:375` (launcher) and `crates/srv/src/backend/wconfig/loader.rs:135` (srv), plus their `extract_commented_setting_key` twins | Each parses the user's file to a map (comments dropped), then re-emits the template with the user's values dropped in. |
| When it writes | launcher `seed_settings_file` -> `ensure_settings_file` (every startup, `sidecar.rs`); srv `merge_settings_to_disk` (every Settings-pane change, `config_watcher_fs.rs:222`); migrations `m0037`, `m0044` | The file is rewritten at **every launch** and at every setting change. |
| Reader | `read_config_file` into the typed `SettingsType`, JSONC (comments and trailing commas stripped) | One value of the wrong type fails the **whole** file: a reload keeps the previous config, a first load runs on defaults. Errors reach `configerrors`, shown in the status bar and the Settings pane. |

## 3. Problems found (why the guards in §7 exist)

These come from reading the code; step 1 of §8 starts with a failing test for each, so none is taken on trust.

1. **A file that does not parse is replaced by the pristine template.** `read_settings_jsonc` and `read_settings_raw_jsonc` return an empty map for any parse failure. `merge_into_template` with an empty map returns the template unchanged, and `ensure_settings_file` writes it. So a half-pasted file, a missing comma, or a stray character is wiped at the next launch, or by the next setting change (which writes the template plus the one changed key). The person loses everything they had set, silently.
2. **A byte-order mark is "invalid".** A file saved by an editor that adds a BOM does not parse (`serde_json` rejects the BOM), so it takes the path in 1.
3. **The user's comments and formatting are lost** at every write: their own comments, a value's trailing note, their order, multi-line values (collapsed to one line).
4. **The write is not atomic and not serialised.** `std::fs::write` truncates then writes. A crash or a full disk leaves a short file. Two writers (the Settings pane and a hand edit, or two quick pane changes) read, change and write without any check, so the later write can undo the earlier.
5. **The two `merge_into_template` copies can drift**, and nothing says they must agree.
6. **Nothing ties the key lists together**: 140 keys in `types.rs`, 187 in the schema, 123 in the template, 65 read by literal name in the frontend alone. New keys (`animation:*`, `agent:previewskidnotches`) were added to some and not others.

## 4. Decisions to confirm (recommendation first)

1. **Source of truth.** Recommend `schema/settings.json`, extended (§5.1). It already holds types, defaults and descriptions, and the defaults test already treats it as the source. The alternative, a new Rust-side registry, moves 187 entries to a new place for no gain.
2. **Template is generated and committed.** Recommend a script (`scripts/gen-settings-template.mjs`, with `--check`, in the style of `gen-rpc-stubs.mjs`) that writes `settings-template.jsonc`. srv keeps embedding the committed file.
3. **A value equal to its default stays set.** Recommend: whatever the person wrote stays; the Settings pane's "Reset" comments the line back to its default. Refresh never changes a set line. (Alternative: the pane comments a line back whenever the value equals the default. That makes future default changes reach people who never meant to pin the value, but it also rewrites lines they set on purpose.)
4. **Leniency for a missing comma.** Recommend strict: a missing comma is an error with its line and column, and the last good settings keep running (§7, case 1). A lenient parser that guesses the comma is a second JSONC dialect to maintain. Say so if you want it anyway.
5. **JSONC edit engine.** Recommend a small span scanner in `agentmux-common` (about 200 lines, no new dependency; `json_comments` is the only JSONC crate in `Cargo.lock` today). Evaluate the `jsonc-parser` crate in the first PR; take it only if it preserves every byte outside an edit.
6. **UTF-16 files.** Recommend: read them (Windows Notepad's old "Unicode"), but refuse UI writes with a message to save as UTF-8, rather than silently converting someone's file.

## 5. Design

### 5.1 One source of truth

`schema/settings.json` keeps its shape. Each property may now carry these extra fields, all optional:

| Field | Meaning |
|---|---|
| `default` | Already there. **Required** for every non-family key, or `x-nodefault: "<why>"` (the dnd:concurrency case: unset means no limit). |
| `description` | Already there. Required (one line shown in the template; the rest stays in the schema and the docs). |
| `x-section` | Which template group and docs section: `Terminal`, `Window`, `Notifications`, ... Required. |
| `x-platform` | `["windows"]`, `["macos"]`, `["linux"]`; absent means all. |
| `x-secret` | `true` for tokens and keys (`voice:groqApiKey`, `messaging:*:token`, ...). |
| `x-example` | For a key with no default, the example shown in the template. |
| `x-renamed-from` | Old key name(s). Drives the migration in §7, case 7. |
| `x-retired` | `"<version>: <note>"`. The key is no longer read; kept for migration and for the guard. |
| `x-since` | The version that added it (for "from a newer AgentMux" notes). |
| `x-unread` | `"<why>"`, for a key listed but read by nothing (the defaults test's `UNREAD` cases). |

Wildcard entries (`app:*`) and per-widget families (`widget:hidden@<widget>`) are **families**: they are not listed one by one in the template; the template carries one comment line per family describing the pattern.

### 5.2 The template (what the file looks like)

Generated, grouped by `x-section` in a fixed order, one block per key:

```jsonc
// AgentMux settings. Saved changes apply at once.
// Every setting is listed with its default. Remove the leading // to override one.
// Template version: 0.60.10        <- a comment, not a key
// Docs: https://docs.agentmux.ai/settings
{
    // -- Terminal --

    // Font size in points.
    // "term:fontsize": 15,

    // Lines kept after they scroll off.
    // "term:scrollback": 2000,

    // [Windows only] Use the legacy console ...
    // "term:somewinsetting": false,

    // Secret: stored here as typed. Never paste this file into a chat or a ticket without removing it.
    // "voice:groqApiKey": "",
    ...
}
```

Rules: the description is the schema's first sentence; a platform key carries `[Windows only]`; a secret key's commented default is `""` (never a real value) and carries the secret note; a key with no default shows `x-example` and says "no default". Keys the user has set appear **uncommented in place**, in the group they belong to; keys the file has that the schema does not know stay where they are (§7, case 4).

### 5.3 The 1-to-1 guard

`scripts/check-settings-keys.mjs`, run in CI and by a vitest wrapper beside `settings-defaults.test.ts`. It extracts, by pattern, every key the code reads, then checks:

| Check | Fails when |
|---|---|
| **Read -> schema** | A key is read in `crates/srv` (struct renames in `types.rs`, `extra.get("..")`, `extra_bool("..")`, named constants such as `desktop_gate::SETTING_*`), `crates/cef` or the launcher, or `frontend/` (`getSettingsKeyAtom("..")`, `settings["..""]`, the Settings pane's controls) but has no schema entry. |
| **Schema -> read** | A schema key (not a family, not `x-unread`) is read nowhere. |
| **Schema -> fields** | A key lacks `default` (or `x-nodefault`), `description`, or `x-section`; a secret key has a non-empty default; a platform value is unknown. |
| **Template drift** | `gen-settings-template.mjs --check` differs from the committed `settings-template.jsonc`. |
| **Docs drift** | The docs repo's settings table (generated from the schema, Opaz) differs from what the schema generates. The docs repo runs the same generator against its `src/agentmux` submodule. |
| **Rename integrity** | A `x-renamed-from` name is still read by code, or a retired key is still read. |
| **Pattern floors** | Each extractor must yield at least N keys, so a pattern that stops matching fails the check instead of checking nothing (the same device the defaults test uses). |

Existing gaps go in `scripts/check-settings-keys.baseline`, each with a reason, and the check fails if a baseline entry is fixed but still listed, so it only shrinks (the `check-file-sizes.baseline` pattern). At the start the baseline holds the 64 schema-only keys and the template-only and frontend-only keys found by the first audit (which the first PR of §8 step 3 writes down).

### 5.4 One writer

A new module `crates/common/src/settings_file.rs` replaces both `merge_into_template` copies and their key extractors. The launcher seed, srv's `merge_settings_to_disk`, and migrations `m0037`/`m0044` call it. It is a **patch**, not a regeneration: it reads the file, scans its top-level members with a JSONC span scanner (key span, value span, the comment lines above it, trailing comma), and applies edits as span replacements. Every byte outside an edit is unchanged.

| Operation | Effect on the file |
|---|---|
| **Set `K = V`** | If `"K": ...` is set: replace only its value text, keep its comments. Else if a commented default `// "K": ...` exists: uncomment it in place with the value. Else: add it under a `// -- Your settings --` block at the end. The last duplicate wins. |
| **Reset `K`** | Comment the line back to `// "K": <default>,`; if there is no default, remove the line. |
| **Refresh (new version)** | Rewrite only the *commented default* lines and their descriptions from the new template; insert keys new in this version at their group; drop commented lines for retired keys. Set lines are never touched. |
| **Seed (no file)** | Write the generated template. |

The file's line endings (LF or CRLF) and BOM are kept; new lines use the file's own ending.

**Never overwrite what it could not read.** Parsing returns `Result`. On any failure (not valid JSONC, a non-object root, not UTF-8 or UTF-16, too large) no write happens at all, not even a refresh. The caller gets the error with line and column.

**Atomic and serialised.** Write to a temp file in the same directory, flush it, then rename over `settings.json` (on Windows, with a short retry when an editor or antivirus holds the file). srv takes one process-wide lock around read-edit-write. The edit is key-level, so when the file changed between our read and our rename (a hand edit), we re-read and re-apply the same key edits, up to three times, then report a conflict instead of writing. A rolling `settings.json.bak` keeps the previous good content, written only when the content changed.

**Last good settings survive a restart.** After each successful load srv stores the parsed settings in `settings.json.lastgood` (atomic). A file that fails to parse at startup runs on `lastgood`, if there is one, otherwise on defaults, with the error in `configerrors`, and is **left exactly as it is**.

### 5.5 Reading: one bad value does not sink the file

`read_config_file` parses to a `serde_json::Value` map first, then validates each known key against the schema (type, `minimum`/`maximum`, `enum`). A key that fails keeps its default and adds a `ConfigError { file, err: "term:fontsize: expected a number, got \"big\"" }`; every other key applies. Unknown keys are kept in `extra` as today. Only a file that is not a JSON object at all (or not parseable) keeps the previous config as now. Validation uses the same schema the guard checks, so the two cannot disagree.

### 5.6 Where the person sees problems

`configerrors` already shows in the status bar and the Settings pane. Each error gains an optional `line`/`column`, and the Settings pane shows a **Fix in editor** link that opens `settings.json` at that position in the Editor (needs Agent1's #16466 to accept a line and column; agreed with Agent1 before the UI step). The Advanced section gets an **Unrecognized settings** list: each unknown key, a "did you mean `term:fontsize`?" suggestion when one is within a small edit distance, and a Remove button. Removing is the only way a key is ever deleted by the app.

## 6. Save and restore by copy and paste

- **Save:** copy the whole file (or just the uncommented lines) anywhere.
- **Restore on another machine, or a later version:** paste it over `settings.json`, save. The result is live at once (the watcher reloads it). Anything that does not belong stays in the file and is flagged, never deleted; anything wrong falls back per key (§5.5); a broken paste changes nothing and says where it is broken.
- **The pasted file's commented lines are from its own version.** On the next launch the refresh (§5.4) brings the commented defaults and descriptions to the running version and leaves every uncommented line alone.
- A later step may add "Copy my changes" to the Settings pane (only the set lines). Not in scope here.

## 7. Edge cases, each with its guard and its test

Every row is a test in the PR named in §8. "Today" says what happens now, from §3.

| # | Case | Today | Behavior after | Test |
|---|---|---|---|---|
| 1 | Invalid JSON, a half-pasted file, a missing comma, an unclosed brace | Wiped to the template at the next launch or setting change. | No write. The last good settings keep running. `configerrors` has the message with line and column and a Fix in editor link. The file is untouched. | A broken file is byte-identical after launch seed, after a setting change, after a refresh. |
| 2 | Byte-order mark (UTF-8) | Counts as invalid, wiped. | Read as valid; the BOM is kept on every write. | A BOM file parses; a set keeps the BOM. |
| 3 | CRLF files, mixed endings | Rewritten as LF. | Endings preserved; new lines use the file's majority ending. | A CRLF file stays CRLF after a set. |
| 4 | Unknown key | Kept (moved to "User Overrides"), no warning. | Kept in place and flagged in Unrecognized settings, with a suggestion. Never deleted by the app. | Unknown key survives set, reset, refresh; appears in the list. |
| 5 | Unknown key from a **newer** AgentMux pasted into an older one | As 4, silently. | As 4; the header's `Template version` is higher than the running version, so the note says "from a newer AgentMux". Known keys with a value the older version cannot read (a new enum value) fall back per key. | A newer-header file loads, flags, and keeps everything. |
| 6 | Wrong type or out of range (`"term:fontsize": "big"`) | The whole file fails: reload keeps the old config, a first load is all defaults. | That key uses its default and reports; every other key applies. | One bad key among ten good: nine apply, one error. |
| 7 | A key was renamed or retired | The old key is orphaned in the file. | `x-renamed-from` moves the value to the new key if the new one is unset, and the old line becomes a comment `// "old": v  (renamed to "new" in 0.60.x)`. A retired key is commented with its note. The value is never lost. | Rename and retire fixtures; value moves once; second run is a no-op. |
| 8 | The user's own comments, order, spacing | Lost at each write. | Preserved by span edits; only the edited value changes. | Bytes outside the edit unchanged, checked on a heavily annotated file. |
| 9 | A multi-line value (an object or array over several lines) | Collapsed to one line. | The value span is replaced whole by a pretty-printed value indented to match; or left alone if unchanged. | A multi-line `connections` keeps its layout on an unrelated set. |
| 10 | A default changes in a new version | The commented line refreshes (the template is re-merged), but so does everything else the person annotated. | Refresh rewrites commented default lines and descriptions only; set lines never change. | Old-default commented line updates; an explicit old-default value stays. |
| 11 | A value set equal to its default | Stays set. | Stays set (decision 3). The pane's Reset comments it back. | Set equals default stays; Reset comments it. |
| 12 | Secrets and tokens | Never in the template; redaction is in `redact.rs`. | Commented default is `""`, never a real value (the guard fails on a non-empty secret default). A value the person sets stays as typed; logs, `configerrors`, exports and the "Copy my changes" output redact it as now. | Guard fixture with a secret default fails; redaction test unchanged. |
| 13 | The Settings pane and a hand edit at once | Last write wins, silently. | Key-level patch re-applied on a changed file (up to three times), then a conflict error. The Editor, when it saves a settings.json that changed on disk since it loaded, offers reload or keep (with Agent1). | Write while the file changes under it: both edits present. |
| 14 | Two pane writes in quick succession | Each reads the file and writes, so one can be lost. | One lock around read-edit-write; edits apply in order. | Concurrent sets of different keys both land. |
| 15 | A crash or a full disk mid-write | A truncated file. | Temp file plus rename: the old file stays whole if the write fails. | Failing writer leaves the original intact. |
| 16 | A huge file or a binary file (or invalid UTF-8) | Read in full; parse fails; wiped. | Over 1 MiB, or containing a NUL byte or invalid UTF-8: not parsed, not touched; error says the file is not a text settings file. | Large and binary fixtures are untouched after launch. |
| 17 | A UTF-16 file with a BOM | Invalid, wiped. | Read; UI writes refused with "save as UTF-8" (decision 6). | UTF-16 file reads; a set is refused and the file is untouched. |
| 18 | Duplicate keys (a block pasted twice) | The last wins in the typed read; the merge keeps one. | The last wins on read and on write; a warning names the duplicates. Nothing is removed. | Two copies: last value applies; set edits the last; warning shown. |
| 19 | A whole generated file pasted *inside* the file (nested braces), or an array at the root | Invalid, wiped. | Case 1: invalid, untouched, clear message ("the file must be one JSON object"). | Nested and array-root fixtures. |
| 20 | Platform-only keys | Not distinguished. | Shown with `[Windows only]` etc.; a key for another platform pasted in is kept, has no effect, and is not flagged as unknown. | A Windows-only key in a Linux-run test is kept and not flagged. |
| 21 | The file is read-only, a symlink, or locked by another program | The write error is only logged; the pane shows the change as saved. | The pane shows "could not save" with the reason; the in-memory change stays for the session. A symlink is written through to its target; a locked file is retried briefly, then reported. | Read-only fixture: error surfaces, content intact. |
| 22 | The file is deleted while AgentMux runs | The next write rebuilds it from the template plus the written key, losing every other live setting. | The next write rebuilds from the generated template plus the settings currently in memory. | Delete then set: the other live settings are written back. |
| 23 | Startup with no file | Template written. | Same, the generated template. | Seed test. |
| 24 | Startup with a file from the previous version | Re-merged. | Refresh (case 10); set lines and comments untouched. | Upgrade fixture. |
| 25 | A key whose value text matters (a path with backslashes, unicode, very large integers, `1e400`, `NaN`) | Re-serialized by `serde_json`. | Value text is replaced only for the edited key, written with `serde_json`; a `NaN` or non-JSON number in the file is a parse error (case 1), not a silent default. | Round-trip a path and a unicode string. |
| 26 | Per-channel and per-instance files | Isolated. | Unchanged; the template version and a pasted file work across channels. | Existing isolation tests keep passing. |
| 27 | A comment containing a key-looking line (`// "term:fontsize": 20` as the person's own note) | Treated as the commented default. | The scanner reads *our* commented defaults only from the generated block pattern (comment immediately above, key in the group); other comment text is left alone. The risk is small; a test pins the behavior. | A user note shaped like a default is not rewritten by refresh. |
| 28 | A migration (`m0037`, `m0044`) writes settings | Calls `merge_into_template`. | Calls the one writer; migrations use the same guards (no write on a broken file; a migration that cannot run records that and retries once the file parses). | Both migrations pass their existing tests through the new writer. |
| 29 | Windows editors that save by delete-and-rename, and fs-watch bursts | The watcher debounces ~300 ms. | Unchanged; our own atomic rename is ignored by the watcher for the same debounce (we know our own write's hash). | Our write does not trigger a second reload. |

Other cases the build should keep an eye on, not decided here: a user who keeps `settings.json` under version control and sees the file change on upgrade (the refresh only touches commented lines, so the diff is comments); a very old file from before the template (the 14-line case in `settings-json-divergence.md`) is handled as any file: refreshed, set lines kept.

## 8. Build plan (one small PR each, in this order)

1. **Stop the wipe (can ship before anything else), in two PRs.**
   - **1a, shipped as #6117:** `settings_file.rs` with the safe write; the launcher seed, srv `merge_settings_to_disk` and the two migrations switch to it; a broken file is never written; BOM and CRLF kept; UTF-16, binary and oversized files left alone; atomic, serialised writes with a re-read when the file changed mid-save. Covers cases 1, 2, 3, 14, 15, 16, 17, 19 and problems 1, 2, 4, 5 of §3.
   - **1b, shipped:** `settings_patch.rs`, a small scanner so a write changes only the edited text: a set value is replaced in place, a commented default is uncommented in place, a new key goes last, a reset puts the commented default back; the commented defaults are refreshed from the template (keys new in the template are added in their section, set lines and the person's own comments never touched). Covers cases 8, 9, 10, 11, 24, 27 and problem 3 of §3. A layout the scanner cannot place falls back to writing the template (a warning is logged), never to losing the write.
   - **1c:** `settings.json.lastgood` and running on it when the file does not parse at startup (case 1).
2. **Per-key validation on read** (§5.5) and line/column on `configerrors`.
3. **Schema fields and the audit.** Add `x-section`, `x-platform`, `x-secret` etc. to all 187 entries; add the missing keys the audit finds; write the baseline. No behavior change.
4. **Generator and generated template.** `gen-settings-template.mjs` with `--check`; the new template (every setting, grouped, described); refresh (§5.4) goes live with it.
5. **The guard** (`check-settings-keys.mjs`, vitest wrapper, CI step).
6. **UI:** Unrecognized settings list, Fix in editor, "could not save" feedback, Editor changed-on-disk prompt (with Agent1's #16466).
7. **Docs table** generated from the schema (Opaz), and the docs repo's drift check.

Each PR runs the checks it touches and names the edge-case rows it covers. The feature is reversible per PR; step 1 alone already removes the data-loss path.

## 9. Not in scope

A settings migration UI beyond the notes above; changing which settings exist or their defaults (`KNOWN_MISMATCHES` stays the defaults test's business); moving settings to another file format; syncing settings between machines (copy and paste is the stated first step); the "Copy my changes" button.

## 10. Open questions for the operator

Only decisions 1-6 in §4, each with a recommendation. If you say "looks good", I take all six as recommended and the build starts at §8, step 1. Step 1 is worth starting even before the rest is settled, because the wipe in §3 problem 1 happens to real files today.
