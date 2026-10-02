# Spec: DRY the no-window spawn flag, and the meta-row type scale

**Status:** proposed

Date: 2026-10-01 · Author: Loap · Status: proposal (not implemented)
Follows: `docs/reports/REPORT_STARTUP_CONSOLE_FLASH_AND_CONTEXT_CARD_SIZE_2026_10_01.md`
Context: the same-day PR fixed two instances (launcher `schtasks`, srv `--crash-monitor`) and the
oversized context-delivery card by hand. Both are the third-or-later occurrence of a class of bug that
a shared abstraction or an inventory guard would have prevented. This spec covers the cleanups.

---

## 1. One way to spawn a child without a console window

### Problem
- `CREATE_NO_WINDOW` already has one definition (`crates/common/src/win32.rs`), but every call site still
  re-writes the same wiring: `#[cfg(windows)] { use std::os::windows::process::CommandExt;
  cmd.creation_flags(CREATE_NO_WINDOW); }`.
  Measured on `main` @ `48f511ca9`: 44 `creation_flags(` calls and 26 `use ...CommandExt` imports
  across the Rust crates, against roughly 125 non-test `Command::new` sites.
- Forgetting the flag is silent. It only shows up as a console flash on a Windows desktop, found by a human
  watching the splash (#2171, #2173, #3788 and the crash monitor all shipped this way).
- Root cause of the exposure: `agentmux-srv` is a console-subsystem binary (no `windows_subsystem`
  attribute; PE subsystem 3). The launcher hides it with `CREATE_NO_WINDOW`, so every unflagged
  console-program child it spawns gets a new console.

### Proposal A: an extension trait in `agentmux-common`
Add `agentmux_common::win32::NoWindow`, implemented for both `std::process::Command` and
`tokio::process::Command`:

```rust
pub trait NoWindow { fn no_window(&mut self) -> &mut Self; }
// windows: sets creation_flags(CREATE_NO_WINDOW) (OR-ing with any flags already set is NOT possible via
//          the std API, so document that callers needing CREATE_SUSPENDED etc. use creation_flags directly)
// other:   no-op, returns self
```

- Removes the per-site `cfg` block and the `CommandExt` import; a call site becomes `cmd.no_window()`.
- `make_cli_cmd` (`crates/common/src/cli.rs:24-37`), `srv/src/util.rs`, and the launcher's new
  `schtasks_command()` helper become one-liners.
- Caveat to handle in the design: `creation_flags` overwrites, it does not OR. Sites that combine flags
  (`host_spawn.rs` uses `CREATE_SUSPENDED`; `cli_login.rs:1331` deliberately uses `CREATE_NEW_CONSOLE`)
  must stay explicit. The trait is for the common case only.

### Proposal B: extend the existing spawn-inventory guard
`crates/srv/src/backend/pane_env.rs` (around lines 210-430) already pins every process spawn in srv with
`SPAWN_INVENTORY: (file, program, count, classification)`, so a new spawn fails a test until someone
classifies it. That mechanism already exists for env sanitising. Reuse it:
- Add a classification, for example `no-window:`, and cross-check that those files call `no_window()` or
  set `creation_flags`, the same way `sanitized:` entries are cross-checked against a sanitizer call.
- Add the equivalent table for the `launcher` crate and the `cef` crate (srv's inventory does not cover
  them; the `schtasks` regression was in the launcher).
- Result: the next `Command::new("something.exe")` in a Windows-reachable crate fails a unit test on CI
  instead of flashing on a user's screen.

### Option C (investigate, do not assume): make srv GUI-subsystem
Marking `agentmux-srv` `windows_subsystem = "windows"` would remove the entire class (its children would not
get a console), but it changes stdio behaviour srv's logging and the launcher's piped-stdio contract may rely
on. Needs a spike: confirm the launcher's pipes and the `--crash-monitor` / CLI modes of the same binary still
work. Listed as the structural fix, not recommended without that spike.

### Order
B first (cheap, catches regressions), then A (mechanical sweep), then decide on C.

---

## 2. Startup does avoidable process spawns

`start_at_login::reconcile_latest` runs on every startup, because the first `config` frame always passes the
dedupe in `observe` (`crates/launcher/src/start_at_login.rs`), and it unconditionally calls
`autostart::read_entry()`, which spawns `schtasks /Query /TN <id> /XML`.

- Opportunity: for the common case (setting off, no task registered) answer from the filesystem instead.
  Task Scheduler keeps task definitions as files under `%SystemRoot%\System32\Tasks\`, so an existence check
  can short-circuit the spawn. Verify the path and ACLs before relying on it (it can be unreadable for
  non-admin users, so fall back to `schtasks` when the check is inconclusive).
- Even with the no-window flag, this is a process spawn and a thread on the startup path for a feature that is
  off by default.

---

## 3. Meta-row type scale in the agent pane

### Problem
- `_document-nodes.scss` has 68 literal `font-size: 11px|12px` declarations (209 across the agent styles) and
  one use of the tokens `--text-xs` (11px) and `--text-sm` (12px) from `theme.scss:333-334`. The context
  card went wrong because a new component set sizes for its small parts and forgot the three that inherit the
  pane base (15px).
- `virtualization/renderers.ts:169-172` encodes the card's rendered heights as magic pixel constants that must be
  kept in step with the SCSS by hand (this PR changed 30 to 24 and 24 to 20 for that reason).

### Proposal
1. Give meta rows (outcome, notice, compaction, memory reinjection, history link, context delivery, markdown
   canceled header) a shared mixin, for example `@mixin meta-row-text`, that sets `font-size: var(--text-xs)`,
   `color: var(--secondary-text-color)` and line-height. New meta components include the mixin instead of
   re-deciding sizes, so the default is "small", not "inherit 15px".
2. Replace the literal 11px/12px in the agent styles with the tokens during that sweep (mechanical; no visual
   change at the current token values).
3. Derive the estimator constants from the same line-height token, or add a test that mounts the card and
   compares its measured collapsed height against `estimateCollapsedContextDelivery` within a tolerance, so a
   style change that moves the height fails a test rather than causing a jump when the row is measured.

### Not in scope
Restyling the other cards. This only prevents the next one from inheriting the wrong size.

---

## 4. Suggested split
- PR 1 (this branch): the two spawn flags and the card sizes, with the estimator constants.
- PR 2: spawn inventory for launcher and cef plus the `no-window:` classification (Proposal B).
- PR 3: `NoWindow` trait and the sweep (Proposal A).
- PR 4: meta-row mixin and token sweep (section 3).
- Separate spike: Option C, and the filesystem pre-check in section 2.
