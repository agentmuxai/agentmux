# Report: muxspect/muxlog cross-channel inspection gaps

Date: 2026-08-22
Status: draft — findings from a live debugging session, not yet a spec

## 1. Why this exists

While investigating a real live bug (a dispatched Agent-tool subagent never
appearing in the user's Swarm pane at all — not a naming issue, an absence),
the debugging process itself turned into a live audit of `muxspect`/`muxlog`:
every step that should have been "run a diagnostic command" instead required
manually walking the filesystem, cross-referencing `Get-Process`, reading env
vars by hand, and guessing which of several candidate log files was the real
one. That manual process is recorded below verbatim as the evidence base for
the extensions proposed in §3. The original bug is still **unresolved** —
see §4.

## 2. Concrete, reproducible gaps found this session

### 2.1 `muxlog swarm` resolved to the wrong (stale) instance's log on the first try

```
$ node ~/.agentmux/shell/muxlog.mjs swarm --grep "ab786c0dbcfa0a121|71a6b2ae" -n 500
=== swarm trace: C:\Users\asafe\.agentmux\logs\agentmuxsrv-v0.55.18.log.2026-08-22 ===
```

`muxlog ls` run moments later showed the actually-freshest srv log was
`agentmuxsrv-v0.55.19.log.2026-08-22` (age `0s`) — a **different, newer**
file than the `v0.55.18` one `swarm` silently picked. `discover()`'s own
sort is `mtime` descending, so on paper `resolveFile("srv", opt)` should
already have preferred the newer file; in practice the wrong one won. This
is a real, reproducible correctness gap in the plain `resolveFile()` path
that `swarm`, `errors`, `auth`, and `bridge` all share.

Contrast with `muxlog phases`, which had this exact class of bug found by
review (see `resolvePhaseFiles`'s own doc comment, muxlog.mjs:450-511) and
was fixed by verifying log **content** actually contains the target block
id, not trusting filename/mtime metadata alone. `swarm`/`errors`/`auth`/
`bridge` never got that fix — they still trust `discover()[0]` (or the
`-i`-filtered first match) unconditionally.

### 2.2 No way to select a log by channel

```
$ node ~/.agentmux/shell/muxlog.mjs swarm -i local-main-b28b7a-b966d418 ...
# no srv candidates matched — exits with "no srv log found matching '...'"
```

`$AGENTMUX_CHANNEL` (`local-main-b28b7a-b966d418` in this case) is the one
identifier an agent or user can always name with certainty — it's in their
own environment. But `-i` only substring-matches against the discovered
file's **path**, **source** label, or **version** string
(`resolveFile`/`muxlog.mjs:335-343`), and srv logs are tagged `source:
"shared"` uniformly (`logRoots()`, muxlog.mjs:30-36) regardless of which
channel's srv process actually wrote them. There is currently no query of
the form "give me the log for *my own* running instance" that's guaranteed
correct — only best-effort filename heuristics.

### 2.3 Multiple srv processes likely share one log file by version coincidence

The resolved `agentmuxsrv-v0.55.19.log.2026-08-22` file contains several
`"subagent watcher initialized"` lines clustered within about 40 minutes
(`18:59:34`, `19:01:29`, `19:01:30`, `19:39:19`, `19:39:20`) — more
initialization events than this one session should produce, consistent with
multiple srv processes (this machine had dozens of `agentmux*` processes
running across portable builds, dev branches, and channels — confirmed via
`Get-Process`) all logging to the same shared, version-keyed file. No log
line carries a channel/instance tag to attribute it to a specific process
after the fact — `muxlog`'s own log-rendering pipeline
(`renderLine`/`shortTarget`) has no such field to show even if one existed.

### 2.4 `muxspect` is explicitly single-instance by design

Per `docs/MUXSPECT.md` and `muxspect.mjs`'s own header comment: Phase 1 only
queries "the instance you're already inside," via
`$AGENTMUX_LOCAL_URL`/`$AGENTMUX_AUTH_KEY` read from the caller's own
environment. `muxspect conversations`/`conversation <agent>` (Phase A of
`SPEC_MUXSPECT_CROSS_TIER_CONVERSATION_VISIBILITY_2026_08_21.md`, merged
just hours before this session) is the first crack at cross-*channel*
visibility, but it only covers registered **agents' conversations** — there
is still no equivalent for "does a different running instance have a
subagent dispatch/controller matching X."

### 2.5 The just-merged cross-channel feature 404s on the actual running instance

```
$ node ~/.agentmux/shell/muxspect.mjs conversations
muxspect: request failed (404): Not Found
```

`describe <my own block_id>` worked fine against the same instance
moments later — so the API path and auth are fine in general, but the
specific route this instance's running `agentmux-srv` binary was built
before `feat(muxspect): cross-tier conversation visibility, Phase A`
merged, and nothing told me that. No `muxspect`/`muxlog` command surfaces
"the srv build you're actually talking to is older than the source tree
you're reading" — every command's output should probably lead with the
live instance's own reported version, and `muxspect`/`muxlog` themselves
could sanity-check that against the newest known feature set instead of
just failing opaquely on the missing route.

### 2.6 No single command answers "where did this dispatch actually go"

The manual reconstruction this session needed — confirm the subagent's
`agent-<id>.jsonl` + `.meta.json` were written to
`projects/<workspace>/<session>/subagents/`, confirm the srv's file watcher
config matches that path/pattern, find the srv log actually written by the
right process, grep it for the dispatch/block id — is exactly the kind of
thing `muxspect`/`muxlog` exist to save someone from doing by hand. Today
neither tool has a "for dispatch/agent X, tell me its full observed
lifecycle" command; `muxlog swarm` gets closest but is a raw filtered tail,
not a correlated answer.

## 3. Proposed extensions

Roughly in the order that unblocks the most downstream value:

1. **Emit a structured instance-identity field on every srv/host log line**
   (e.g. `channel=<AGENTMUX_CHANNEL>` or a short stable instance id), not
   just in specific recipes' matchers. This is the actual root fix — `-i`
   filtering, `phases`' bespoke content-verification resolver, and every
   other gap in §2.1-2.3 are all workarounds for not having this at the
   source. Once present, `muxlog -i` can match it directly instead of
   guessing from file paths.
2. **Factor `resolvePhaseFiles`'s content-verification logic into a shared
   resolver** all recipes call, instead of only `phases` having it.
   `swarm`/`errors`/`auth`/`bridge` inherit the same
   wrong-instance-resolution risk today.
3. **A real "list live instances" command** — extend `muxlog ls` (or add
   `muxspect instances`) to cross-reference actual running processes (PID,
   port, channel, version — ground truth) against the log files discovered
   on disk, rather than inferring liveness from log mtimes alone. This
   would have made the very first step of this session's investigation
   ("which of these processes/channels is the one I'm actually in")
   instant instead of manual.
4. **`muxspect` Phase 2 cross-instance querying** (already planned per
   `SPEC_MUXSPECT_LIVE_INTROSPECTION_TOOL_2026_08_01.md`) — when it lands,
   prioritize exactly the query this session needed: "which running
   instance(s), if any, have a controller or subagent dispatch matching
   block_id/agent X" — discoverable without the caller already knowing the
   target channel.
5. **Surface the live instance's own version prominently** in every
   `muxspect` command's output (not just buried in `list`/`describe`'s
   process metadata) so a stale-build 404 like §2.5 is self-diagnosing
   ("you're talking to v0.55.19, this route needs v0.55.20+") instead of a
   bare 404.
6. **A correlated dispatch-lifecycle command** —
   `muxlog swarm --dispatch <id>` or `muxspect dispatch <id>` — that
   productizes the manual workflow in §2.6: find the subagent's
   transcript/meta files, confirm the watcher's config covers that path,
   pull the matching srv log lines (via the shared resolver from #2), and
   report a single verdict (written → watched → registered → visible, or
   the exact step where it stopped).

## 4. Resolved — the original bug

**Root-caused 2026-08-22, once §3.1/§3.3/§3.6 landed, exactly as predicted
above.** `session_belongs_to_block` was NOT the cause — that gate never even
ran. `muxlog ls`'s LIVE column plus `muxlog swarm -d <id>` (both from this
report) made it possible to confirm, on a real live dispatch, that
`subagent_watcher`'s own "watching for subagent JSONL files" event fired
exactly once, at agent registration, against
`~/.agentmux/shared/providers/claude/projects` — while the dispatch's real
transcript was written under
`~/.agentmux/shared/identities/<uuid>/claude/projects/...` (the agent's
actual bound Armory identity). Two different directories; the watcher could
never have seen the write no matter how long it waited.

The cause: `subagent_watcher::resolve_claude_config_dir` trusted the
block's persisted `cmd:env.CLAUDE_CONFIG_DIR` — a write-once, launch-time
snapshot of the *generic* shared-provider dir. `SPEC_PROVIDER_ISOLATION_2026_06_20.md`
§4.3 already documented that this snapshot goes stale for any
identity-bound agent, and re-resolves the REAL value on every turn via a
separate code path (`inject_identity_env_with_broker`) that never writes
back into `cmd:env`. `subagent_watcher` was added later and never joined
that re-resolution. Fixed in
`docs/specs/SPEC_SUBAGENT_WATCHER_IDENTITY_BOUND_CONFIG_DIR_2026_08_22.md`.
