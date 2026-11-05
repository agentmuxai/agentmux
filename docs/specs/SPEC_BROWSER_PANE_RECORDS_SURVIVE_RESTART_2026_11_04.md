# SPEC: A limited, agent-owned browser pane keeps its limit and its owner across an srv restart

**Status:** implemented — in the PR that carries this file (Kanban #20857).
**Author:** Korp@narko, 2026-11-04. **Follow-up to:** the restore-loop fix (#6206).
**Builds on:** `SPEC_BROWSER_PANE_ALLOWED_ORIGINS_2026_10_09.md`, `SPEC_AGENT_DRIVEN_BROWSER_PANES_2026_10_07.md` §3.

## 1. The problem

srv holds two records in memory: which agent owns an agent-opened browser pane (`browser_owner`), and
the sites each limited pane's chain may go to (`browser_allowlist`). After an srv restart both are
empty while the pane's block still carries `browser:owner_agent` and `browser:allowed_origins`. A
limited pane becomes unlimited, and its agent can no longer drive it, with the header still showing
both. AgentY's call (2026-11-03): not intended; a limited pane and its owner must survive a restart,
and srv and the host must not disagree.

## 2. Why the block's meta alone is not enough

A duplicated pane copies the meta under a new block id. The owner record says a duplicate must not be
agent-owned, so rebuilding owners from the meta of every browser block at startup would hand a copy
to the original's agent.

## 3. The change

- A file in the config folder, `browser-pane-records.json`, keyed by block id, holds the owner of each
  owned pane, each limited pane's chain root, and each chain's site list.
- It is written whenever either record changes (the host sync loop already wakes on every change),
  skipped when nothing differs from the last write, and read once at startup, before the first
  change and before the host's first push.
- A record comes back only if the block is still there, is a browser pane, and its
  `browser:owner_agent` names the same agent. Only srv writes that key, and a person's Take over
  clears it, so a taken-over pane stays taken over. A duplicate has a new id, so it has no record.
- A chain's list comes back with its remaining panes, including popup panes that outlived their root.
- A closed pane leaves the file with its block (the existing block-gone hook drops it from the
  records, and the next write removes it); a block deleted while srv was down is dropped at load.
- Unchanged: the 24-hour "lost to a Take over" record, and a pane's grants and pauses, keep their own rules.

## 4. What did not change

The host's copy is still pushed from srv's records (`browser_host_sync`), now including the restored
ones from the first push. The host's settled-window guard against a stop-and-reload loop
(#6206) is unchanged; a pane woken with a list srv no longer has is still reported and cleared.

## 5. Tests

`browser_pane_records`: a record returns only for a block that is still a browser pane its agent
still drives; a copy under a new id gets nothing; a limited popup outlives its closed root; restored
records drive `owner_of`, `list_for` and the navigation refusal; the file round-trips and an empty
file reads as empty.
