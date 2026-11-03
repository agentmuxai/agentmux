# Report: what stopped an agent finishing a task in a browser, 2026-11-03

**Author:** QTest@narko, at the operator's request ("agents need wide autonomy, write a record of the issue, and other blockers too, agents also need to be able to enter passwords"; "coordinate with agenty to get it done, after we finish the app store submission").
**Status:** record only. Nothing here is fixed yet. The first item is the one the operator asked to fix.
**Rule it comes from:** the operator's 2026-10-25 direction, "agents can do literally anything, except put words in the operator's mouth": a new "an agent can't type this" guard is a flag in review unless it stops a forged operator voice, and "if an agent can't do something today, build an agent API for it".

## What happened

The operator asked an agent to finish a US tax form (a W-9) and a bank account in a web console, signed in as the operator in AgentMux's browser. The agent had the numbers (they are in the operator's own private record). It filled every ordinary field, then could not type the taxpayer ID or the bank numbers. Each time it had to stop and ask the operator to type, or to approve a one-time prompt, for a value the operator had already told it to use ("you have the EIN"). A form that needed about four minutes took several round trips.

## The rule the change follows (for the operator to read before any code)

Set with AgentY on 2026-11-03; the change in item 1 does exactly this and nothing wider.

1. **Allowed:** a `fill` with `text` or with a `vault` entry into a password, one-time-code, taxpayer-ID, card or bank field, when the agent already holds the value. Holding it means one of: a Vault entry it was granted (`vault`); the operator's own record or message that the agent was given for the task (`text`); a value the person typed for it earlier (`request_value`, unchanged).
   **What the first change does (2026-11-03):** `text` into those fields, in a pane opened with `allowed_origins`; `vault` into a taxpayer-ID, card or bank field already worked and is unchanged; `vault` into a password or one-time-code field stays refused for now (a saved sign-in has `signin`), a follow-up if wanted.
2. **Audited as the agent, never as the person:** every such fill writes an audit row with the agent, the pane, the page's origin, the field kind (password, one-time code, taxpayer ID, card, bank), and the value source (`vault:<entry id>`, `text`, or `person`). **Never the value, and not its length.** The snapshot and any read-back keep showing the field as filled with the value hidden; the tool answer says "filled" and the kind, never the value. The audit is the operator's way to see what an agent typed where.
3. **Refused (the prompt-injection guard kept):**
   - the field's page origin is not one the pane was opened for: a pane is limited to the `allowed_origins` the agent gave when it opened it (a page going elsewhere already asks the person), so a field on any other origin is refused and says so;
   - the field is inside an iframe whose origin differs from the top page's, unless that origin is also in `allowed_origins` (the usual way a look-alike form is slipped in under a trusted page);
   - a `vault` fill whose entry names a site or service that does not fit the field's origin keeps its existing check (the person is asked where it may go), so a Vault secret still cannot be sent to an unrelated site;
   - a value that came from the page itself (text read off the page, an instruction in it) is not a value the agent holds: page content is untrusted, so an agent must not type a secret because the page said to. That is the agent's own rule; the tool cannot tell, so the audit's value source is what makes it checkable afterwards.
4. **Not changed:** a committing click (Submit, Agree) still waits for the person's approval in the pane. That is the person's act, not a typing guard, and it stays whatever this change does.
5. **How "the origin is one the agent is working on" is decided:** by the pane's `allowed_origins` (exact origin, or `*.domain` for subdomains, https), the same list that already limits navigation. No new list. A pane opened without one (the person's own tabs) keeps the refusal.

## 1. Agents are refused any password, taxpayer-ID, card or bank field (the one to fix)

- **Where:** `crates/cef/src/browser_api/act.rs` (the refusal in `fill`: "this is a password, one-time-code, card or Social Security / taxpayer ID field ... AgentMux never lets an agent type into one") and `crates/cef/src/browser_api/snapshot.rs` (the `[secret: ask the user to fill this; ...]` mark on such a field, chosen by the field's name, label or type).
- **What it does:** `fill` with `text` is refused on those fields. The two ways round it are `fill` with a `vault` entry (the value never reaches the agent, and the person approves where it may go) and `request_value` (the person types it, once, and can keep it in the Vault). The tool descriptions also say never to fill a password or a one-time code: the person types those.
- **Why it blocks:** the operator is the person who is asking. An agent that already has the value, from the operator's record or the Vault, still can't use it without the operator typing it again. A password for an account the agent was told to use is the same: an agent can't sign in for them.
- **What to change (for discussion, not decided here):**
  - allow `fill` with `text` on those fields when the agent has been given the value for the task (the operator's own words, a Vault entry, a record), recorded in the audit as the agent that did it;
  - let a Vault entry fill a password or a one-time code field (today the tool text forbids it), still without the value reaching the agent;
  - keep what the guard was for: the value is never returned in a snapshot or a read-back, never logged, and the pane keeps asking the person to approve a committing click;
  - a field the page marks as a password or card number keeps its mark in snapshots (so the agent knows), but the mark says "filled from a Vault entry" or "allowed", not "ask the user".
- **Not to be worked around:** an agent that has the value must not set it with a script (`eval`) to get past the guard. The change has to be the guard's, so the audit and the "never shown to the agent" rule stay true.

## 2. Other blockers met the same day

| # | Blocker | Effect | Where / note |
|---|---|---|---|
| 2 | A committing click (an agreement's Agree, a form's Submit) waits for the person to approve it in the pane | right for a legal or irreversible click, but it also gated the harmless "View" click that opens the agreement; each wait is a round trip if the person is away | the pane's approval of committing clicks; consider a "read only" class that is never gated |
| 3 | `request_value` dismissed once by mistake, with no way to see the prompt again except to ask again | one extra round trip | the answer says only "the person said no"; it could say how to be asked again |
| 4 | A click's result is stale after a re-render: "the page changed since the snapshot (the element is gone)" on a button that had in fact worked | the agent could not tell whether the Agree click had gone through; it had to reload and read the status | `click` could say what the page did next (the dialog closed, the status changed) instead of failing |
| 5 | `Browser close` on a pane the person keeps answers "its user didn't keep it in time" | unclear whether the pane closed; a closed pane the person wanted open | the wording; the 30 s keep window applies to a pane the agent itself opened |
| 6 | A pane the agent drove in an earlier session looped (restore, white, steals focus) and AgentY had to tell it to close it | the agent could not name the pane (a new session has no pane id) | `Pane layout` shows no owner for a browser pane; show who drives it so any agent can close its own |
| 7 | Not an agent API problem but a tooling one: `cargo nextest` is not installed on narko, so a test run that falls back silently ran no tests | a green-looking "exit 101" read as a run | the repo's check scripts; say which runner ran |
| 8 | The merge rules are strict but the signals are noisy: CI "cancelled" jekts arrive each time a comment or review lands on the PR, so it is not clear whether to re-run | time spent checking whether CI had run at all | the CI job's concurrency group cancels the PR run on an unrelated event; the cancel jekt is not a failure |

## 2b. Notes added while the rest of the task was finished (same day)

What the later steps showed. The task got done, but only because the operator was at the keyboard.

| # | Blocker | Effect | Note |
|---|---|---|---|
| 9 | A taxpayer ID, a routing number and an account number each needed a `request_value` prompt that the person had to answer, one after another | three prompts for values the agent already had; the first was dismissed, so the agent stopped and asked in chat | the same cause as item 1; the prompt's label already carried what it was for |
| 10 | An attestation checkbox ("under penalties of perjury", "I have read and agree") showed as `[checked]` before the agent touched it, in three dialogs | the agent could not tell whether the person ticked it or the page pre-ticks it; it matters because a certification is the person's own act | a snapshot cannot say who changed a control. The pane could mark a control the person changed, and an agent should never tick an attestation itself |
| 11 | The last button of a dialog (Add, Agree) was gone by the time the agent looked, with no way to read who pressed it or whether it was pressed | the agent had to reload and read the page's status to learn the result (agreement accepted, bank account "Processing") | `click` and `wait` could return the page's next state, or `Browser log` could list the clicks the person made in the pane |
| 12 | `wait` with `until_text` on a dialog's text returned an HTTP 400 error after its timeout, and on an earlier call returned "done" at once while the dialog was still loading | polling with `find` instead; each is a call and an error line to read | a timeout is an answer, not an error; and `until_text` should look inside a dialog |
| 13 | Refs change after every step in a multi-step dialog (Account Holder, Bank Details, Certification), and a scope by `dialog "name"` only works while that dialog is the current one | a find or snapshot before every action | refs could be stable for the same control across a step, or a step's controls could come back in the click's own answer |
| 14 | A "Same as Legal Entity" checkbox re-rendered the whole dialog and cleared the fields filled just before | work redone | a page behaviour; the answer to the click could say the form was reset |
| 15 | The operator's rule (2026-11-03): no Rust build or test on narko, and `task dev` only when the operator must see it | a UI fix cannot be checked live in a dev build on this machine; the live check has to be offloaded or skipped | offload the check to charlie or Area54 (or run only `vitest`, which is allowed) |
| 16 | Money and tax pages that sit behind a long processing time ("banking updates are processing, ... 24 hours", Paid Apps Agreement "Pending User Info") | the agent can't tell when the task is finished, and a session is gone by then | an agent needs a way to be woken for "check this page in a day" (Schedule exists; this should be the habit) |

## 3. Proposed order

1. Item 1 first (the operator's ask): the `fill` guard, with the audit and the no-echo rule kept. Owner: the browser seat (Korp), with QTest as requester; AgentY to assign.
2. Items 2, 3, 4 with it, since they live in the same tool (`Browser`) and the same file set.
3. Items 5 and 6 are small wording and layout changes.
4. Items 7 and 8 are CI and tooling and go to the platform seat.

A test for item 1 should fail on main: a `fill` with `text` into a field marked as a taxpayer ID, with the value given for the task, succeeds, appears in the audit under the agent's name, and the value is absent from the snapshot and the read-back that follow.

## 2c. Notes added during the EU trader (DSA) step (same day)

| # | Blocker | Effect | Note |
|---|---|---|---|
| 17 | A pane id is lost when a session ends, and a new session opens a second pane on the same console | the agent starts again from the sign-in state; nothing says an earlier pane of its own is still open | `Browser open` could answer "you already have a pane on this site" with its id |
| 18 | `snapshot` right after a `click` that navigates returns the old page with the new URL | the agent reads a stale tree and takes a second snapshot | `click` with `snapshot: true` could wait for the navigation to settle |
| 19 | `snapshot` with `scope: dialog` (a bare role) fails with HTTP 400; it needs the dialog's name | a failed call to find the name first | a role alone could mean "the topmost dialog" |
| 20 | The public contact details Apple will publish (phone, email) are a choice only the person can make, and the record has none for the company | correct to stop; but the agent could not tell what was missing without opening the form | the agent should ask for the choice as soon as it knows the form has the field; the record should list the company's public contact details once chosen |
| 21 | Apple's states moved faster than its own text said (bank "up to 24 hours", Active in a few hours) | a scheduled check became unnecessary | a scheduled check should read the state first and cancel itself when done |
