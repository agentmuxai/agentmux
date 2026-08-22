# SPEC: `muxlog swarm -d/--dispatch` — a correlated dispatch-lifecycle verdict

**Date:** 2026-08-22
**Status:** Implemented
**Author:** Korp
**Repos touched:** `agentmux` (`agentmux-srv/src/backend/shellintegration/muxlog.mjs`)
**Related:** Ext 6 of `docs/reports/REPORT_MUXSPECT_MUXLOG_CROSS_CHANNEL_INSPECTION_2026_08_22.md`

## 1. Problem

The report's own motivating investigation — a dispatched Agent-tool
subagent that never appeared in Swarm — needed to answer one question
repeatedly: "did `subagent_watcher` ever log anything about this specific
dispatch, on the instance I'm actually resolved to?" There was no single
command for that. Answering it meant: confirm which instance/channel you're
actually looking at (§2.1's whole motivation), run `muxlog swarm`, and
manually eyeball a long trace for a matching id — with a silent "nothing
printed" looking identical whether that meant "genuinely never processed"
or "your `--grep` didn't match the right field."

## 2. What ships

`-d`/`--dispatch <id>` (any position, like every other muxlog option):

1. In `renderLine`, filters on the **raw line text**, before/regardless of
   JSON parsing — not the rendered message. A dispatch/subagent id can
   appear either in the message text or as a bare structured-field value
   (`dispatch_id`, `session_id`, `agent_id`, ... the exact field name
   varies by call site in `subagent_watcher.rs`); `--grep` only ever
   matches the message field (documented behavior, see `phases`'s own doc
   comment), which would silently miss a field-only occurrence. A plain
   substring check on the whole raw line catches both without needing to
   enumerate field names.
2. Combined with `swarm` specifically, prints an explicit **verdict** line
   after the trace: `N lines mention '<id>' in <file>`, or, on zero
   matches, a longer explanation with next steps (check `muxlog ls` /
   pass `-i` explicitly — you may be resolved to the wrong instance; or
   try a shorter/partial id substring). A silent zero-line result looks
   identical to "nothing to show" for every other filter combination;
   `-d` gets a verdict because "never appeared" is exactly the symptom
   someone reaches for this flag to diagnose, and an empty result IS the
   answer, not an accident to leave ambiguous.
3. `printLastLines` now returns the total match count (before the `-n`
   cap) instead of nothing, so the verdict is exact — "5 lines matched, 2
   shown" is derivable, not guessed from "did stdout print anything."

`-d` composes with every other option exactly like `--grep`/`--level`/etc.
already do (AND, not replace) — e.g. `muxlog swarm -d <id> --level warn`
narrows to just the warning-level lines about that one dispatch.

## 3. Non-goals

- **Does not correlate against the subagent's own transcript/meta files**
  (`~/.claude/projects/<workspace>/<session>/subagents/agent-<id>.jsonl` +
  `.meta.json` — the filesystem-side half of the manual correlation the
  report's investigation did). `muxlog`'s own domain is `~/.agentmux`
  logs; it has no existing knowledge of Claude's separate config directory
  layout, and inventing that discovery here would be a much larger,
  differently-scoped change than a log filter. Left as a genuine gap —
  confirming "was the file even written" still needs a manual check.
- **Does not query live registry/Swarm state** (is this dispatch currently
  visible in the running Swarm UI right now) — `muxlog` is history (log
  files), not live state; that's `muxspect`'s domain, and no existing
  `muxspect` endpoint describes one dispatch by id today (would need new
  backend work — a real follow-up, not attempted here).
- **Does not attempt fuzzy/partial id matching automatically** — a caller
  passing a shortened id gets literal substring semantics like every other
  filter here; the zero-match verdict's own text suggests trying a
  shorter substring by hand rather than the tool guessing at truncation
  itself.

So this closes the **log-correlation** third of the manual workflow the
report described (file written → watcher config → **log line** → registry
state) — the most directly achievable piece within `muxlog`'s existing
scope — not the full four-signal correlation. A genuinely complete
"describe this dispatch's full lifecycle in one shot" tool would need the
filesystem-side check and a new `muxspect` endpoint too; scoped out here
as real follow-up work, not silently implied to be covered.

## 4. Testing

- `renderLine`'s `--dispatch` filter: 5 cases (message-text match,
  field-only match, no match, composes with `--level`, unaffected when
  unset) — pure, no I/O.
- `printLastLines`'s return value: 2 cases (count exceeds the `-n` cap;
  zero matches) against a real temp file.
- 34/34 passing in this file's full suite.
