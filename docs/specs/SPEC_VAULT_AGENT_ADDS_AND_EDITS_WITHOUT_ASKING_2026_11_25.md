# Spec: An agent adds or edits a Vault entry without asking the person; the history line is the record

**Task:** #48656 (the build is #48657). **Author:** Agent2-6 (Vault). **Date:** 2026-11-25.
**Status:** active — approved 2026-11-25 with every recommended answer; built in three PRs (§11): PR 1 (adds and edits ask nobody, the pace, the unseen mark) is merged, PR 2 (details kept, a login's place) is merged, PR 3 (the pane: the strip, Undo, Seen, Accept and Dismiss, the row marks, the use card's line) is merged, and PR 4 (the removal of the old card paths from `UseRequest`) is the last.
Written against origin/main `d7ebfaba77`. It changes
`SPEC_VAULT_AGENT_ASKS_TO_BIND_A_LOGIN_TO_A_SITE_2026_11_19.md` (#48141): §7 says how. It builds on
`SPEC_VAULT_PARITY_ENTRY_HISTORY_REASONS_AND_IMPORT_2026_11_06.md` (history lines, reasons, import) and
`SPEC_VAULT_REQUESTS_AS_CARDS_IN_THE_AGENT_PANE_2026_11_01.md` (the cards).

## 1. The requirement

The Vault asks the person before an agent may add some entries and before it may change some others. That stops: an agent
that can add or change an entry does it, and the record of it is the entry's history line (which agent, why, from where,
which version). The person is told afterwards, in the pane, and can undo it; they are not stopped beforehand. What an agent
does with a *value* later (types it into a page, runs a command with it, shares it into a browser profile) is a different
act and asks as it does today. Every rule below is a rule about a change being recorded or about a use; none is a
gate on who may add or edit.

## 2. Every add and edit an agent can make, today and under this spec

From the code (`vault_import.rs`, `vault_import_batch.rs`, `vault_rotate.rs`, `browser_capture.rs`, `shell_secrets.rs`,
`vault_inspect.rs`, `vault_handlers.rs`, `inspect.rs`, `uses.rs`). "Silence" is what happens when nobody answers a card:
`goes_ahead` lets a use of an API key, login or note go ahead after 30 s, and never lets an import, a batch, a new place
for a machine copy, a share or a new account.

| # | An agent does | Today | Under this spec |
|---|---|---|---|
| 1 | `generate`: a new API key, login or note whose value the Vault makes | no card | unchanged |
| 2 | `request`: the person types a value the agent does not have | a card in which the person types | unchanged: there is nothing for the agent to add; it asks for what it lacks |
| 3 | `import` a new entry from AWS Secrets Manager | card, silence is a no | **no card** (R11 for the value) |
| 4 | `import` with `id`: replaces an entry's value | card ("replaces"), silence is a no | **no card** (decision 1; R11) |
| 5 | `import_batch`: a named list, new or replacing | one card with a row each, silence is a no | **no card**; one history line per row |
| 6 | `Browser` `capture` into a new entry | no card | unchanged |
| 7 | `Browser` `capture` into an existing entry | card ("replace with what the page shows"), silence goes ahead after 30 s for a key, login or note | **no card** (decision 1) |
| 8 | `Terminal` `run` with a `capture` rule, new entry | no card | unchanged |
| 9 | `Terminal` `run` with a `capture` rule, existing entry | card naming the command, same silence rule | **no card** for the replacement; the command's own use of secrets is a use (§4) |
| 10 | `rotate`: a new value the Vault makes, set at the service by the agent's command | card naming the command | **unchanged** (decision 4): the command also receives the entry's current value |
| 11 | `update` label, plain fields, tags, expiry | no card | unchanged |
| 12 | `update` `source_*` (where the machine copy is) and `readers`, `rotated`, `copies` (notes the entry keeps about its key: who reads it, when it last changed, where else it is kept; free text, at most 8 lines of `readers`; no field in this row grants a use to anyone) | card, silence is a no | **no card** (decision 2) |
| 13 | `update` `site`, `realm`, `proxy`, `profile`; `generate` with the same | refused: "the person sets it in the Vault pane" | allowed on a login the agent itself made; recorded as a proposal on any other (§6) |
| 14 | `update` `shared`, and `share` for another agent or for every agent | refused ("the person's to share") | unchanged: who may *use* an entry is a use (§4) |
| 15 | delete, empty a Vault, change a Vault's agents or its sign-in Vault | no agent action | unchanged: there is none, and none is added |

Two things look like edits and are not: `Browser` `generate_password` (a new account at a site) and the change of a
password at a site (`browser_change`). Their card is about acting on the *site* with a password the Vault makes, a use of
the browser. They are unchanged. The entry they write gets its history line as today.

## 3. Requirements

**R1. The change is made, then recorded.** For rows 3, 4, 5, 7, 9, 12 and the immediate part of 13, no card is shown, no
attention-list item is made, the call does not wait for the person, and the answer to the agent carries no "needs your
yes" outcome. Every other check stays: the agent's proven identity, the Vaults it may use (an entry in a Vault it is not
let into is "no entry"), the kind's rules (`validate`), the compare at the write that refuses a change when the entry
moved since it was read, and the AWS grant a container agent's read runs under (the broker's own policy and its hourly
cap, never the card).

**R2. The reason stays required.** Every add and edit, including each row of a batch, needs a reason of 8 to 300
characters on one line (`history::require_reason`). An empty one, a short one, one over 300, and one made of whitespace
or control characters is refused before anything is written, with the words it has now. The reason is shown in the
history line and the strip (§5) as the agent's words, quoted and never as markup.

**R3. The history line is the record, and it is complete.** One line per entry written, in the same transaction as the
change (a change with no line is not made), carrying: the agent (`agent:<id>`, as `actor_label` writes it), the action
(`generated`, `imported`, `captured`, `rotated`, `changed details`, and the existing ones), the entry, the Vault, what
changed in names only (`label`, `field site`, `tags`, `value`, `machine copy`), where it came from (the AWS secret's name
and region, the page's origin, the command's first 160 characters; never a value, a tail or a hash), the reason, the
references, the version the value is on after it, and the machine it happened on. The line is chained as every line is. A
refused call writes a line too when it got as far as an entry (as `denied` lines do now).

**R4. A change keeps what it replaced.** A value replaced keeps the old value for 7 days (`db_vault_versions`, unchanged).
A change to details (label, fields, tags, machine copy) keeps the details it replaced the same way: one row per change
with the previous label, fields and tags, which are plain and never secret, for 7 days, in the same transaction.

**R5. Never a value back.** None of these calls returns a value, a tail, a length or a hash of one. The answer names the
entry, the version and the history line's number, and says what was written ("changed: field source_kind").

**R6. A pace, not a permission.** Each agent has a budget over the actions of rows 3 to 13: by default **30 writes per
hour**, of which at most **6 replace an existing entry's value** (at most **5 per entry**), and **20 new entries per
hour**. A call over the budget is refused at once with the time the budget next has room, writes a refused line, and
changes nothing. A batch counts one write per row. The numbers are constants in one place. The budget resets by itself;
nobody has to answer anything for it.

**R11. A value an agent had the Vault read is not trusted until the person has seen it.** An `import` (new or replacing)
reads with the host's own AWS setup: the AWS command-line tool runs as the person's user, with the person's profiles,
and the agent names the profile (`--profile`) from the ones on the host. That is not shown to be no more than the agent's
own shell reads, so the card's "silence is a no" cannot simply be dropped. Instead: a value written by an agent's
`import` or `capture` is **unseen** until the person has marked its change seen or has answered a use card for the entry
once. While it is unseen, a *use* of it never goes ahead on silence (`goes_ahead` is false), offers no *Always*, and the
card says "its value was set by <agent>, <when>; you have not looked at it". The same rule as R9, applied to the value.
The *Always* grants the entry already holds are dropped when the value is marked, because a use on a grant asks nobody.
Container agents are unaffected in what they can read (the grant); they get the same rule for the value. A value the
Vault made (`generate`, `rotate`) is not unseen: nobody had it before.

## 4. What still asks, and why

Only a **use** of a value asks. The requirement names adding and editing; a use is neither.

- A fill into a page, a sign-in, a command run with a value in its environment (`Terminal` `run` with `secrets`), a share to
  a browser profile (`share`, and `import` with `share`: the share is the use, so its tick stays on a card), a reveal
  or copy in the pane, a read of a CI secret with the person's own GitHub login. All as today: the same cards, the same
  30 s silence rule (a login, key or note goes ahead; an ID, a card, a drone's use and a share never do), plus R9 and R11.
- `rotate` (row 10): `set` receives the new value in `NEW_SECRET` **and the entry's current value in `OLD_SECRET`**, so it
  is a command run with a value, the same as `Terminal` `run` with `secrets`, and it keeps its card, and its "Always" for
  two command lines. It is free only if `OLD_SECRET` is no longer passed (decision 4).
- A new account at a site and a change of a password at a site (above).
- What the agent has not got: when it lacks a value, `request` asks the person to type it.

**Where the line is hard to draw (the operator's choice, §8):**
1. *Replacing an existing entry's value* (rows 4, 7, 9). It is an edit, and it can break a working login. The old value
   is kept 7 days and restored in one click. This spec makes it free, because the requirement says edit and the undo is
   cheap, and puts it first in the strip (§5); R11 holds the first use.
2. *Changing where an entry's machine copy is* (row 12). It decides what "same" means in a later `check`; `check` answers
   only same, different or could not read. This spec makes it free and records the place before and after by name.
3. *A login's site* (row 13): §6.

## 5. What the person sees instead of a card

No modal, no prompt, no sound, and nothing in the attention list. Two places in the Vault pane, both quiet:

1. **A strip at the top of the entries list**: "3 changes by agents since you last looked. Review". It shows when there is
   at least one change by an agent the person has not marked seen, and says nothing otherwise. Review opens a list,
   newest first, one row per change: the agent, the time, the entry's label, what changed in words ("added", "new value",
   "place of its machine copy"), the agent's reason quoted, and **Undo** and **Seen**. A proposal (§6) has **Accept** and
   **Dismiss** in place of Undo. A replacement of a value on an entry the person made, and any proposal, sort first and
   carry a plain word ("replaced the value of an entry you made", "wants to move this login to <origin>; it keeps working
   at <old origin> until you accept"). *Seen all* clears the strip. The "seen" mark is per machine and never synced.
2. **The entry's own history** (the existing tab) shows the same lines, and **Undo** appears on any change by an agent
   inside the 7 days: restore the earlier value (the existing Restore), or put back the earlier details (R4). An agent's
   add is undone by removing the entry the way the person removes one today. An undo is itself a history line made by the
   person.
3. **The row of an entry** an agent added or changed since the person looked carries a small "new" or "changed by
   <agent>" mark until seen.
4. **On a use's card** (an unchanged card), the line of R9 and R11 when the value or the site is the agent's and unseen.

A digest (an hourly or daily notice outside the pane) is not built: the strip is enough. If the operator wants it, it is a
setting off by default, in a later PR.

## 6. A login's site, and how an agent-made login gets one

A login with no site is offered to nobody and cannot be shared (`shares::check_shareable`). Today `generate` of a login with
a site, `import` of a new login with `fields.site`, and `capture` of a login (the page's origin is the site, unless the
agent gives one) give a site at creation. `update` refuses to set it later, and refuses `profile`, `realm`, `proxy` and
`shared`.

**R7. Creation takes a site.** `capture`, `import` and `generate` of a login accept `site` (an exact origin: scheme, host,
optional port, no path, query, user information or wildcard, the import's rule `target_origin(site) == site`), and the
login is made for the asking agent's own browser profile (srv derives `profile` from the proven identity, as `generate`
does; the agent never names one). `capture` of a login with no origin of its own (a download, a copied value) takes
`site` from the agent.

**R8. Later, `update` may set `site`, `realm`, `proxy` and `profile`, with a reason and a history line, in two ways.**
- On a login **the asking agent itself made** (its first history line is that agent's), the change takes effect at
  once. `profile` may be set to the agent's own, and only on such a login that has none: a login usable in an agent's
  browser is a share by another door, so on any other login that is `share`, with its card (§4).
- On **any other login** (one the person made, or another agent did), the change is recorded as a **proposal**: the
  call returns at once, saying "recorded as a proposal; it takes effect when the person accepts it", the login keeps its
  old site, realm, proxy and profile in every way (the person's own sign-ins at the old site keep working), and the strip
  shows it first with **Accept** and **Dismiss** (§5.1). A proposal waits 7 days, one per entry and field; a new one
  replaces the old; Accept applies it only if the entry is as it was when proposed (the compare at the write). It is a
  record in a small table (`db_vault_proposals`: entry, field, value, agent, reason, time), never a card, and the call
  never waits.

Origin rules and warnings: a site must pass R7's check. An `http`, loopback, private or look-alike origin is accepted
(a development site is a real case) and is marked as such in the strip row and on every use card for it, with no
*Always* offered for it (#48141's R12 stands).

**R9. A site an agent set is not trusted until the person has seen it.** After an agent sets or changes `site`, `realm` or
`proxy` on a login it made, a *use* at that place never goes ahead on silence, and offers *Always* only after the person
has marked the change seen or has answered a use card for that place once. The card says "the site was set by <agent>,
<when>". A grant held for the old origin stops matching (its key names the origin) and the list shows it.

**R10. `shared` stays out of `update`.** Who may use an entry is a grant of use, and a use asks (`share` is the card). It is
not an edit.

## 7. Where it fits

- **#48141 (bind a login to a site, one click).** Mostly replaced, and what is asked of the person becomes one click in
  the strip: an agent proposes the site of a login it did not make (R8); the person's Accept is the approval. Its card,
  its waiting-card count and remembered no go away. What stays: the origin rules (its R2.5), the warnings about a
  development, private or look-alike origin (its R4, now on the strip row and the use card), *Always* withheld for such
  origins (its R12), and the share tick (its §4). Its decision 8 (changing a site that exists) is answered by R8's
  proposal. #48142 is re-pointed to those parts or closed by the operator's answer.
- **Vault sync (#48347 and the stack that merged).** A change an agent makes is an ordinary edit on its machine and
  travels as one; the history line travels as an activity record. The "seen" mark and a proposal do not travel (a proposal
  is the machine's own). An Undo is a new edit with the person's line.
- **Linked and backed entries.** A backed entry (AWS Secrets Manager holds the value) has nothing to replace; its edits are
  rows 11 and 12.
- **Drones.** A drone has no add or edit action; its uses keep "silence is a no".
- **Container agents.** The AWS grant they read under is unchanged and is the limit on what they can import.

## 8. Decisions for the operator

Shortest first, each with a recommendation.

1. **Replacing an existing entry's value without a card?** Recommend **yes, free**, with the old value kept 7 days, Undo,
   the strip's first row, R11 for imported and captured values. Alternatives: a card only when the entry was made by the
   person, or a card always (the status quo).
2. **Changing where a machine copy is without a card?** Recommend **yes, free**; the history line names the place before
   and after. The alternative keeps today's card for this one row.
3. **A site an agent sets: at once on a login it made, a proposal on any other?** Recommend **yes** (R8, R9). Alternatives:
   creation only (R7), or keep the refusal.
4. **Rotate.** Recommend **keep the card** (its command receives the current value as well as the new one). The alternative
   is free rotate with `OLD_SECRET` no longer passed, which breaks a service whose change needs the old secret.
5. **The pace.** Recommend 30 writes an hour, 6 replacements an hour, 5 per entry, 20 new entries (R6).
6. **The strip and no digest.** Recommend **the strip only** (§5).
7. **Details are kept 7 days like values** (R4), so a changed label or place can be put back.

## 9. Gates

No operator-only gate is added by any of this: every rule is about a change being recorded or about a use. The ones that
exist today and go: the refusal of `site`, `realm`, `proxy` and `profile` for an agent (`inspect::person_only`, and
`generate`'s copy of it), and the cards of rows 3, 4, 5, 7, 9 and 12. The reserved tags AgentMux sets itself stay (they are
the system's), and `generate`'s refusal of ID and card kinds stays (the Vault cannot make a number; `request` asks for one).

## 10. Tests the build adds

Rust (`agentmux-srv-core`, `agentmux-srv`): each of rows 3, 4, 5, 7, 9 and 12 makes its change with no `UseRequest`
created and no wait, and writes one history line in the same transaction (agent, action, entry, names of what changed,
source, reason, version, machine); row 10 still asks; a call that fails after the entry was read writes a refused line;
the reason is refused when empty, short, over 300, whitespace, or control characters, for a batch row too; the pace
(R6): the 31st write, the 7th replacement, the 6th replacement of one entry and the 21st new entry are refused with the
time of room, each counted per agent, a batch row each, and the window slides; a details change keeps its previous label,
fields and tags and Undo puts them back, a replaced value is restored as today, a version past 7 days is not; an agent
cannot read a value from any of these answers; a container agent's import still fails when its grant does not name the
profile; a native agent's import runs the AWS tool with the sanitized environment it has now (a test of the allowlist),
and an unseen imported or captured value never goes ahead on silence and offers no *Always* until seen or answered once
(R11), while a generated or rotated one does as today; `update` on a login the agent made sets `site`, `realm`, `proxy`
and, when it has none, the agent's own `profile`, never another agent's or `personal`, never `shared`, and rejects a path, a
query, user information and a wildcard; on any other login it writes a proposal and changes nothing else (the old site
still offers the login), a second proposal replaces the first, a proposal past 7 days is gone, Accept applies it only
if the entry is as it was, Dismiss drops it; R9 as for R11, and a grant held for the old origin stops matching;
`goes_ahead` is unchanged for every other use and for a drone. The old card paths (`import`, `batch`, `link` and the
replacement path of capture) are removed from `UseRequest` and the generated stubs; the reactive-coverage and RPC-stub
checks pass. TypeScript (vitest): the strip shows a count and hides at zero, its rows sort replaced-value changes and
proposals first, Undo, Seen, Accept and Dismiss work, the agent's reason is shown as text with markup shown as text, *Seen
all* clears it, and nothing in the strip opens a modal; the entry row marks; the use card's line appears only for an
unseen value or site set by an agent. A look check on macOS and Linux (strip, rows, the mark, the history's Undo) at two
zoom levels, light and dark.

## 11. The build split

Three PRs, in this order, each small enough to review:

1. **The Vault stops asking for adds and edits (Rust).** Rows 3, 4, 5, 7, 9 and 12 lose their cards; R1, R2, R3, R5, the
   pace (R6) and R11. The card paths stay in the type until PR 3, so the frontend keeps compiling.
2. **Details are kept, and an agent may set a site (Rust).** R4, R7, R8 (with the proposals table), R9, R10; the use card's
   new line as data.
3. **The pane (TypeScript).** The strip, the review list, Undo, Accept and Dismiss, the row marks, the use card's line, the
   removal of the dead cards and their stubs, the dictionary entries; the macOS and Linux look check.

The tool description for `Vault` and the agent-facing docs change in PRs 1 and 2 with the behaviour.

**PR 2 as built.** R4 (the details an edit replaced, kept 7 days with the change; the pane's Undo for them is PR 3), R8 (a login the
agent made takes `site`, `realm`, `proxy` at once and its own `profile` where it has none; any other login gets a proposal
that Accept applies only if the login is as it was, and Dismiss drops), R9 (`set_what` on the use request names a value or a
site an agent set; marking drops the entry's *Always* grants), R10, and R7 for `generate` (a site is one exact origin; realm and
proxy may be given) and for `import` and `capture` of a login with a site (the place is marked unseen). Deriving the asking
agent's own profile for an `import` or `capture` of a login is not in this PR.

**PR 3 as built.** The strip above the entries list (a count that hides at zero, Review, *Seen all*), its list (proposals and a
replaced value of an entry the person made first; Undo, Seen, Accept, Dismiss; a removal is confirmed in its row; the
agent's reason is shown as text), the mark on an entry's row, Undo in the entry's History for a replaced value or details
inside their 7 days, and the use card's line for a value or place an agent set and the person has not looked at. The seen
marks are on this machine only. The removal of the dead card types from `UseRequest` and their cards is left to a last
small PR, so this one stays reviewable.

**PR 4 as built.** The batch and link cards are gone from `UseRequest`, with their types, the batch rows and Vault choice of an
answer, their cards, styles, words and tests: nothing can ask them since an agent's imports, captures and edits ask nobody.
What stays is the question of an import that shares the new login with the agent, a rotation, a share, a sign-in, a fill and a
new account.
