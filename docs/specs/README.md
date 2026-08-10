# docs/specs/

Draft specs, design explorations, and implementation plans. These are work-in-progress documents that haven't been formally approved yet.

Once a spec is approved and ready for implementation, move it to the top-level `specs/` directory.
Once implementation is complete, move it to `specs/archive/`.

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
