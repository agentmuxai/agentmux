# docs/specs/

**Every spec lives here.** There is no second spec tree — a doc's lifecycle
state is its `**Status:**` line (below), not the directory it sits in.

`archive/` is the one subdirectory, and it means "not worth reading unless you
are doing history", not a lifecycle stage. Status still applies inside it.

### Why there is no `specs/` tree anymore

There used to be a top-level `specs/` for "approved / being implemented", with
`docs/specs/` as drafts and `specs/archive/` as done. That worked as a promotion
workflow and then quietly inverted itself. Measured before the trees were
merged (2026-09-01):

| Tree | Documented as | Actually held |
|---|---|---|
| `specs/` | active and approved | 51 `draft`, 2 `implemented` |
| `docs/specs/` | drafts, not approved | **126 `implemented`**, 14 `active` |
| `specs/archive/` | completed or superseded | 7 `draft`, 3 `ready` |

An agent trusting those descriptions looked in exactly the wrong tree. Worse,
promoting a file between trees broke every code comment citing it, silently:
**32 of 165 spec citations in source were already dangling**, 9 of them
pointing at `specs/X` for a file that had already moved to `docs/specs/X`.

Directory-as-status and the `Status:` field were two answers to the same
question, and only one of them is enforced (`scripts/check-doc-status.sh`) or
kept current. So the directories stopped being an answer.

**Do not reintroduce a second tree** to mean approved, active, or done. Set the
Status line instead. If you want to find every implemented spec, read
[`INDEX.md`](./INDEX.md), which is generated from Status.

## Status field

This is the actual rule, not a suggestion (docs-lifecycle hardening Phase 1 —
see `SPEC_DOCS_LIFECYCLE_HARDENING_2026_08_03.md`):

Every spec carries a `**Status:**` line whose **first word** is one of the
closed enum:

```
draft | proposed | active | implemented | living | historical | superseded
```

- `draft` — still being written; not yet ready for review.
- `proposed` — written and reviewable; nothing has shipped.
- `active` — partially implemented / implementation in progress. The line
  **must say what shipped (PR #s) and what remains.**
- `implemented` — shipped. The line **must cite the implementing PR(s).**
- `living` — continuously maintained reference; has no terminal state.
- `historical` — records a past state or effort; kept for the record, not a
  plan for anyone.
- `superseded` — replaced by another doc. **REQUIRES** a `**Superseded-by:**`
  line pointing at a real path in this repo (a broken pointer is worse than
  none).

After the enum word, a dash and free text is encouraged — evidence, dates,
PR numbers, what remains. Example:

```
**Status:** active — Phase 0 shipped in PR #2394; Phases 1-6 not started. Verified 2026-08-10.
```

### Reader guardrail

Statuses rot (this doc's parent spec found shipped features still marked
"no code yet" 48 hours after landing). Before trusting a spec's Status — or
any checkable claim in it (a file path, "Phase N done", "PR merged") —
**spot-verify against current code.** A Status line is a claim about the
code; nothing re-verifies it automatically once written.
