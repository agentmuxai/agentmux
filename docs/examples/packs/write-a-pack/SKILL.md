---
name: write-a-pack
description: Use when asked to write, review or update a pack: the per-application knowledge an agent loads before it drives a desktop app (Word, Ableton Live, WaveLab or any other). Tells you the format, the control ladder, and how to keep unverified steps out.
---

# Write a pack

**Status:** active — the guide for writing a pack (spec: docs/specs/SPEC_PACKS_2026_11_05.md).

A **pack** is per-application knowledge for an agent that drives a desktop app: how to control it, best route first, its keys, its traps, and a check after each step. The format is an Agent Skill (`SKILL.md`) plus a small manifest. Spec: `docs/specs/SPEC_PACKS_2026_11_05.md`. Examples: `docs/examples/packs/word`, `ableton-live`, `wavelab`.

## What to write

A folder named for the pack id, with:

- `SKILL.md`: the frontmatter (`name`, and a `description` that names the app and its executable so an agent knows when to load it), then the ladder, shortcuts, pitfalls, a verify table and the proof line. Keep it short; an agent reads all of it.
- `pack.json`: `schema` `pack/v1`, `id`, `name`, `version`, `app` (vendor, `executables` per platform, `titleHints`), `platforms`, `ladder`, `mcp` (a catalog id, never a command), `verified` (empty until a test passes).
- `test.json`: the pack's test task, its steps and the verify cues, written so a machine can check them (a file, a window title, a script result).

## The ladder, in this order, in every pack

1. The app's own API or scripting.
2. A plug-in or MCP server (named by catalog id; the person approves its install).
3. Keyboard shortcuts and menu paths.
4. `Desktop` clicks, with a screenshot after each action.

## Rules

- **Only steps you saw succeed go in as fact.** Anything taken from documentation, a forum or memory stays, but says where it came from and that it is unverified. A pack with no passing test says "Verified on: none yet".
- **A pack grants nothing.** Never write "the grant is not needed". Say the person's per-app grant is needed.
- **Never put a password, key, licence code or account name in a pack.** Tell the agent to ask the person.
- **Say what is destructive** (overwrites, sends, plays audio, deletes) and what to check before it.
- **Name the fallback and when to stop.** If the first route fails twice, say which route is next, and when to ask the person.
- **No claims you cannot check.** If you could not read a source, say so in the pack, as WaveLab's does.

## Versions

- One pack per app, never a copy per app version. `SKILL.md` holds what is true for every version; a line that differs starts with a range tag, `[v >=21]`, `[v <21]` or `[v >=20.1 <21]`; an untagged line is true everywhere. Use `≈` when you saw it on one version and do not know how far back it goes.
- Check that a call, property or menu item exists before relying on the version number, and say so in the pack.
- Every saved change bumps `version` and adds a `changelog` entry. A test result is recorded with the pack version and the app version it ran on.
- A newer app than any tested one still gets the pack, with the warning "untested: verify every step".

## Drafting from a session

When you explore an app for the pack: record each action you took and what you saw after it (the `Desktop` audit log keeps the actions). Put into `controls` only the actions whose result you confirmed. A person reviews the pack before it is saved.

## Proof

Run `test.json` yourself with the person's grant on a machine that has the app, then append `{appVersion, platform, date, runner, result, notes}` to `verified`. "Verified on Live 12.1" means exactly that: the test passed on 12.1.
