# RCA: running the tests of a PR opened terminal windows on a person's desktop

Date: 2026-10-28. Status: the test helpers are fixed and the inventory is tightened (this PR). One part of the mechanism is still unexplained (section 5).

## 1. What happened

On a Windows 11 development machine, an agent running `cargo test` for a change that starts `git` and `bash`
many times caused bursts of terminal windows on the person's desktop: about a dozen at a time, one every few
seconds. Some closed at once; the ones whose program had exited with an error stayed open until closed by hand.
The first window was titled with the path of Git for Windows' `git.exe`.

## 2. Cause

- The tests' own helpers started `git` and `bash` without `CREATE_NO_WINDOW` (`.no_window()`): the `git()`
  helper in `crates/bashwrap/src/git_hooks.rs`, and three tests in `crates/bashwrap/src/shim_functions.rs`
  that run `bash -c`. Windows Terminal is the default console, so a console program started this way gets a
  Windows Terminal window, and Windows Terminal keeps the window open when the program exits with an error
  code.
- The two bursts line up with the shared build slot's history: each run of those tests opened 7 to 9 windows,
  about one every 3 s. Runs of other agents' tests in the same period opened none.
- **Why nothing caught it:** `crates/common/tests/spawn_no_window_inventory.rs` pins every process spawn, but its
  entry for `git_hooks.rs` was classed `mixed:` ("one is a test helper's git"), and its `test:` class says
  "never on a user machine". Tests do run on a person's machine, so the exemption was wrong.

Shipped code was not affected: every spawn outside the tests in those files already calls `no_window()`.

## 3. Fix

- The four test spawns call `no_window()`; the Windows-reachable `git` helper in `claude_layout.rs`'s test
  support did not either, and now does.
- The inventory no longer accepts `test:` or `mixed:` for `git`, `sh` or `bash`: a spawn of those is
  `no-window:` (a test helper hides it too) or `not-windows:` (a unix-only test). The unix-only rows were
  reclassified `not-windows:`.

## 4. The build-slot shim is not the cause

The `cargo` shim is a bash script that runs the slot script, which runs the real `cargo`, with plain `exec` and
a direct call (no `setsid`, `nohup`, `start` or detached child; the heartbeat is a background shell
subshell). A test that asks the console API about itself, run through the shim in the same shell, reported a
console window handle and ten processes attached to the console, so the test process has a console like any
other child, and an unflagged child would inherit it.

## 5. What is still open

Tests that start `cmd`, `ping` or `sh` without `no_window()` exist elsewhere (the inventory's `test:` rows) and
ran in the same period without opening windows, so "no flag in a test helper" does not by itself explain the
windows. What differed for the `git` and `bash` children is unproven. A test spawning `git` through the
`cmd\git.exe` redirector, without the flag, on a machine where a window is harmless (not the person's
desktop) is the way to settle it. Until then the rule is the cheap one: every spawn is hidden, tests included,
and `git`, `sh` and `bash` are enforced by the inventory.
