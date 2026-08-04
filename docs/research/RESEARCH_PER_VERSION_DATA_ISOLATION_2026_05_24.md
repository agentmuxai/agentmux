> **⚠️ SUPERSEDED — 2026-06-13.** Retained for its design rationale and the inbound code/doc references that cite it. For the current, code-anchored architecture of agent data & cross-channel persistence, see **[ARCHITECTURE_AGENT_DATA_AND_CROSS_CHANNEL_2026_06_13.md](../architecture/ARCHITECTURE_AGENT_DATA_AND_CROSS_CHANNEL_2026_06_13.md)**.

# Research: User-data continuity across versions in desktop apps

**Date:** 2026-05-24
**Author:** AgentA (Claude Opus 4.7)
**Context:** AgentMux currently isolates user data per build version (`~/.agentmux/versions/<version>/{data,agents,config,...}/`). This is safe (you can't corrupt v0.38.4's data by running v0.38.5) but creates real UX friction in the active-development workflow: every `task package` bump produces a portable with an empty "My Agents" list, and side-by-side comparison of builds means re-creating the test agent each time.

The existing [`SPEC_DATA_DIR_UNIFICATION_2026-05-05.md`](../specs/archive/SPEC_DATA_DIR_UNIFICATION_2026-05-05.md) acknowledged this trade-off (line 103: "Version-comparison testing (running 624 alongside 639) requires re-doing every login") and chose isolation as Phase 1, with `shared/` as a Phase 2 escape hatch for cookies. Schema-safety was the explicit motivation. This research surveys what other desktop apps do, identifies the pattern they converge on, and proposes a concrete next step for AgentMux.

---

## 1. The trade-off, stated precisely

There are three forces in tension:

1. **Schema safety.** A new version's DB writes can corrupt an older version's reads if you share the file. Migrations are not always reversible.
2. **User continuity.** Conversations, agent definitions, identity, memory, sessions are valuable. Forcing the user to rebuild them on every install is a tax.
3. **Side-by-side install.** Power users (and devs in particular) want to run multiple builds simultaneously without cross-talk.

The naive solutions each fail at least one of these:

| Naive approach | Schema safety | Continuity | Side-by-side |
|---|---|---|---|
| Single global data dir | ✗ (new version can corrupt) | ✓ | ✗ (running both blows up) |
| Per-version data dir (today's AgentMux) | ✓ | ✗ (fresh start every bump) | ✓ |
| Per-instance data dir (UUID-keyed) | ✓ | ✗ (worse — fresh start every launch) | ✓ |

The interesting question is: how do well-engineered desktop apps thread this needle?

---

## 2. Industry reference patterns

### 2.1 Chrome / Chromium — **channel-based, single dir per channel, in-place migration**

Chrome runs as four channels: Stable, Beta, Dev, Canary. Each channel has its **own** user-data dir:

- Stable: `~/Library/Application Support/Google/Chrome/`
- Beta: `~/Library/Application Support/Google/Chrome Beta/`
- Canary: `~/Library/Application Support/Google/Chrome Canary/`

**Within a channel**, every Chrome update writes to the same dir. The Profile has a `Preferences` file with a `version` field; on launch, Chrome runs forward migrations on the profile if `version < current_version`. There is no per-build-number isolation — updating Chrome 130.0.6723.69 → 130.0.6723.70 is invisible to the user, data-wise.

**Side-by-side** is the cross-channel install: you can run Stable + Canary simultaneously because they're separate channels with separate dirs. Running two builds of the same channel is **explicitly not supported** (single-instance enforcement).

**Migration philosophy:** forward-only, idempotent, automatic on first launch of a newer build. If a profile is from a *newer* Chrome than the running binary, Chrome refuses to open it (read-only safety lock) rather than risk a downgrade-corrupt.

### 2.2 VS Code — **channel-based, profile-versioned, with explicit import**

Two channels (Stable, Insiders), each with its own data dir:

- Stable: `~/.vscode/` + `~/Library/Application Support/Code/`
- Insiders: `~/.vscode-insiders/` + `~/Library/Application Support/Code - Insiders/`

Within a channel, settings, extensions, workspace state all persist across updates. Extensions self-migrate via their own version field. The platform itself runs DB migrations on the SQLite-backed state on first launch.

**Cross-channel import:** "Insiders" prompts on first launch to import from Stable. One-time, opt-in.

### 2.3 JetBrains IDEs (IntelliJ, PyCharm, ...) — **versioned dir + import wizard**

JetBrains is the **outlier** that chose per-version dirs by default:

- `~/Library/Application Support/JetBrains/IntelliJIdea2023.3/`
- `~/Library/Application Support/JetBrains/IntelliJIdea2024.1/`

But they bridge the continuity gap with an **import wizard**: on first launch, the IDE detects prior-version dirs, lists them, and offers "Import settings from IntelliJIdea2023.3." Cherry-picks settings, keymaps, plugins, recent projects. Default-on. The user clicks once.

This is the **closest analogue to AgentMux's current state** plus the missing piece (an import wizard).

### 2.4 PostgreSQL — **explicit migration via `pg_upgrade`**

Postgres major versions (15 → 16) keep separate data dirs by default. Same-major (15.3 → 15.4) is in-place. To move major versions you run `pg_upgrade` explicitly, which does either a link-mode (instant, in-place) or copy-mode (slow, safe) migration. Old dir is preserved until you delete it.

**Key principle:** the version number's structure encodes compatibility expectations. Patch/minor = in-place. Major = explicit migration with rollback path. This is the most disciplined version of the same idea Chrome uses implicitly.

### 2.5 macOS / iOS native apps — **single per-app sandbox, in-place forever**

`~/Library/Application Support/<bundle-id>/` is **the** data dir. There is no per-version isolation. App developers are expected to ship forward-migrations with every update, and Apple makes it trivial via Core Data's migration framework (lightweight migrations, custom migration policies, mapping models).

When this works it's invisible. When it fails (rare in production, common in dev) you get a corrupt store and the app crashes on launch. There is no in-product mechanism for the user to "roll back to v3 because v4 broke my data" — they're expected to restore from Time Machine. This is essentially Chrome's model with no parachute.

### 2.6 Docker / k8s persistent volumes — **explicit data/binary separation**

Not a desktop app, but the cleanest articulation of the underlying principle: **the binary is replaceable, the data is not.** Volumes survive container replacement. Image updates are casual; data migrations are deliberate. The discipline is enforced by the architecture, not by the developer's care.

---

## 3. The pattern they converge on

Stripping away surface differences:

1. **Compatibility is a property of versions, not of installs.** Versions in the same compat band (Chrome's "minor update", Postgres's "patch", VS Code's "Stable update") share data. Versions across compat bands don't.

2. **Channel ≠ version.** The unit of isolation is the **channel** (release train), not the build number. Within a channel, in-place migration. Across channels, separate dirs and optional import.

3. **Forward-only, automatic migration within a channel.** No prompts, no choices. The user shouldn't know a migration happened. Schema version is in the DB; migrations run idempotently on launch.

4. **Forward-compatibility safety lock.** If the on-disk data is from a *newer* version than the running binary, refuse to open (read-only or hard error). Don't try to "downgrade" — that's where corruption lives.

5. **Import-on-first-launch when channels are crossed.** Detect prior-channel data, offer to import. One-click, one-time, with backup.

6. **Snapshot before migration.** Copy the file/dir to `.bak` before running any non-trivial migration. Costs a few hundred MB once; saves a recovery story.

7. **Export/import as user-controlled escape hatch.** Even if everything else works, give the user a way to dump and reload their state. Doubles as a portable-across-machines story.

---

## 4. Where AgentMux sits today

Mapping AgentMux against the pattern:

| Principle | AgentMux today | Gap |
|---|---|---|
| Compatibility = property of versions | Every build is its own compat band (per-version dir) | All builds are pessimistically treated as incompatible |
| Channel ≠ version | "Channels" exist conceptually (Dev / Portable / Installed) but only Dev is branch-keyed; Portable + Installed are per-version | Portable + Installed need a channel concept |
| Forward-only in-place migration | No migration code at all (spec §3.4: "no migration") | Need a migration framework, even a trivial one |
| Forward-compat safety lock | None | A newer-version DB opened by older srv would just crash mid-query |
| First-launch import | None | The Phase 2 escape hatch (`shared/`) was deferred |
| Snapshot before migration | N/A (no migrations) | Trivial to add once migrations exist |
| Export/import | None | No way to move agent state between machines or across major-version-equivalent breaks |

The spec author made the right Phase-1 call (isolation is the safe default; migration code is expensive). What's missing is the **Phase 2** that makes the model livable.

---

## 5. Recommended path forward

Three increments, each shippable independently:

### 5.1 Increment A: Channels (the structural fix)

Introduce an explicit `channel` concept distinct from `version`:

```
~/.agentmux/
├── channels/
│   ├── stable/                  ← every released Portable + Installed build of any version
│   │   ├── data/                ← single objects.db, sagas.db, ... ; schema-versioned in-DB
│   │   ├── agents/              ← single shared agent workspace pool
│   │   ├── config/
│   │   └── logs/
│   ├── beta/                    ← reserved for future beta channel
│   └── dev-<branch>/            ← what's currently dev/<branch>/, renamed for symmetry
├── shared/                       ← unchanged from current spec §3.1
└── snapshots/                    ← auto-created backups (see §5.2)
    └── stable-pre-vX.Y.Z.bak/
```

Within `channels/stable/`, every version reads and writes the same DB. The current `versions/<version>/` layout disappears for shipped builds; dev keeps its branch-keyed dirs.

**This breaks the current "every portable is its own world" guarantee.** Two portables of different versions can't both be running at the same time (within a channel) — single-instance enforcement applies, just like Chrome. The user already accepts this for `task dev`.

**For the active-dev workflow** (which is what triggered this research): create a `local-<branch>-<hash>` channel that mirrors stable but is meant for the user's own test builds. Portables built via `task package` for personal smoke-testing land there. The bot can opt into `local-<branch>-<hash>` channel by default and the user gets continuity across local build iterations.

### 5.2 Increment B: Migration framework + snapshots

Add a `schema_version` row to each SQLite file's `meta` table. On srv launch:

1. Read `schema_version` from `meta`.
2. If `schema_version == code_version`, proceed.
3. If `schema_version < code_version`:
   1. Copy DB file to `~/.agentmux/snapshots/<channel>-pre-v<code_version>-<timestamp>.bak`.
   2. Run forward migrations in order. Each migration is a `static const` SQL string in code, applied in a transaction.
   3. Update `schema_version`.
4. If `schema_version > code_version`: **refuse to open.** Log a clear error: "this data was written by AgentMux vX.Y.Z; you're running vA.B.C — please upgrade or use a separate channel."

Migration code lives in a single `migrations/` directory in `agentmux-srv`. Each migration is its own file, named by target version. New schema = new file; never edit a shipped migration.

Snapshots are auto-pruned (keep last 5 per channel).

### 5.3 Increment C: Import wizard (cross-channel + cross-major)

On first launch of a fresh channel (or after a major version bump), detect prior-channel data and prompt:

> "Import agents and identity from <prior-channel>? This won't affect <prior-channel>."

One-click. Copies (doesn't move) the relevant DB rows + agent workspaces. Records the import in `meta` so the prompt doesn't fire again.

This is JetBrains's pattern, applied at channel boundaries instead of version boundaries.

---

## 6. What to NOT do (anti-patterns observed)

- **Don't add timestamps to `versions/` dirs to dedupe.** That's recreating the per-instance dir model, which solves nothing.
- **Don't allow concurrent writes to the same DB from two versions.** SQLite WAL doesn't save you from schema-incompatible writes. The single-instance lock is load-bearing.
- **Don't ship migrations as "edit the DB in place on launch with no backup."** That's the macOS model. It works in production. It does not work during active development. The snapshot is cheap.
- **Don't let `task package`'s auto-bump fight the channel model.** If patch bumps share a data dir, that's fine. The version label on the portable folder is for the *operator's* memory of which build is which, not for the data layer.

---

## 7. Concrete next step

Smallest move that delivers immediate value:

**Ship Increment A (channels) as a single PR with one new channel: `local-<branch>-<hash>`.** Local builds via `task package` land there by default (no production user is affected). Within `local-<branch>-<hash>`, all builds share one data dir. The "My Agents" list survives the next `bump patch && task package`. Total scope:

- `agentmux-common/src/data_paths.rs` — add channel parameter, derive instance_dir as `channels/<channel>/` for the `Portable` mode when built with a "local-<branch>-<hash>" flag (or env var).
- `Taskfile.yml` — `task package` sets `AGENTMUX_CHANNEL=local-<branch>-<hash>` in the bundled binaries.
- No schema migration code yet — within a single channel without released versions, just trust the schema. Migration framework can wait for Increment B.

Test:
- Bump to v0.38.6, build portable, launch.
- Create an agent "test-continuity-1".
- Bump to v0.38.7, build, launch.
- Confirm "test-continuity-1" appears in My Agents.

Once that lands, Increment B unblocks a real channel rollout to Installed + (eventual) Stable.

---

## 8. Open questions for the user

- **Is the production install model in scope, or only the dev/portable workflow?** Increment A only addresses local-build continuity if scoped to `local-<branch>-<hash>`. Channels for shipped builds is a bigger conversation (release process, update flow).
- **Should `task dev` participate?** Currently keyed on branch. Could optionally collapse all dev branches into a single channel, but the per-branch isolation is genuinely useful for parallel feature work.
- **What's the right blast radius for an accidental "wrong channel" launch?** Worst case today: lost test agents. Worst case under §5.1: same. Worst case under §5.2 with a buggy migration: corrupted shared state — hence the snapshot, hence the safety lock.
- **Does this become a `RFC #...` in the existing reducer/state-machine arc, or its own track?** Adjacent but not the same conversation. Suggest its own discussion.

---

## 9. References

- [`docs/specs/SPEC_DATA_DIR_UNIFICATION_2026-05-05.md`](../specs/archive/SPEC_DATA_DIR_UNIFICATION_2026-05-05.md) — current state + Phase 1 design
- [`agentmux-common/src/data_paths.rs:81-117`](../../agentmux-common/src/data_paths.rs) — current resolution logic
- Chrome user-data-dir docs: https://chromium.googlesource.com/chromium/src/+/HEAD/docs/user_data_dir.md
- VS Code "Insiders import" issue thread: github.com/microsoft/vscode/issues/?q=is%3Aissue+insiders+import+settings
- JetBrains "Import settings from previous version" docs: jetbrains.com/help/idea/migrating-from-a-previous-version.html
- PostgreSQL `pg_upgrade` docs: postgresql.org/docs/current/pgupgrade.html
- Apple Core Data lightweight migration: developer.apple.com/documentation/coredata/using_lightweight_migration
