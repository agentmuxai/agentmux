# Spec: fold Reviews into the Kanban: a card shows what its pull requests wait on, and the Reviews pane goes away

**Status:** active. Approved 2026-11-22; the choices in section 10 were taken as recommended. Built in Kanban #48402, in the three pull requests of section 11.
**Date:** 2026-11-22
**Author:** Camper@narko, for Kanban #48401 (build: #48402)
**Written against:** `origin/main` 31c769e45a, by reading code; nothing was run. The owners of the Reviews design and of the card's pull request store were asked and their answers are folded in.
**Changes:** `SPEC_REVIEWS_PANE_2026_11_15.md` (its status line says where each part went, section 9).
**Neighbours:** the Kanban's pull request work (a card keeps all its pull requests; unlink, add and the unmatched list), the Swarm's task line (#48355), the deep-links framework (#48140).

## 1. Requirement

The Kanban alone carries what the Reviews pane was for: for each card, what its open pull requests wait on, their review verdict, checks and queue state, and the detail behind them. The Reviews pane is removed. Nothing the Reviews design built on the server is lost.

## 2. What exists

- **The Reviews pane (R1)** reads `GET /api/v1/reviews`, a list srv builds by joining the Kanban cards to the review, checks and merge flow events it receives. Only the pane calls the route; it is open to agents and container agents, with a query for "which of my pull requests wait on me".
- **The one rule.** `waits_on()` (`crates/srv-core/src/backend/reviews.rs`) is a pure function of the merge state, the checks state, each reviewer's verdict (stale or not) and the task's state, with a table-driven test. It is the only place the rule lives and stays so.
- **The join.** Flow events name a repository by its numeric id only, so they match a card by the holding agent and the pull request number; a repository id learned from one unambiguous match settles a later clash. (The Reviews spec's section 6.1 says "repository and number"; that is wrong and the status-line update corrects it.)
- **The card now** keeps all its pull requests (`prs`) with state, `review`, `checks` and `followed`, and a `pr_rollup`, built in one batched pass per board by the store. The store's rows have review and checks columns that nothing writes yet; the flow events are kept only in memory (the last 100 of each kind), so they are gone on a restart.
- **Not there:** per-reviewer detail, findings by priority, queue position and required checks need an authenticated GitHub read (the Reviews design's phase R2), which does not exist and needs a credential source decided first.

## 3. Data, in srv

- **Persist on arrival.** When srv accepts a review, checks or merge notice it writes the result into the matching card pull request row (verdict per reviewer kind with the head it was given on, checks state with its head, merge or queue state, time). A card then carries them across restarts, and nothing reads the in-memory lists at card time.
- **The join** is the existing one: holding agent plus number, with the repository id learned only when exactly one linked pull request of that agent has that number. Two cards of one agent holding the same number in different repositories stay ambiguous and show no chips until a notice or push event settles it: never a guess.
- **Which commit is current.** The newest commit any notice or the pull request's own report named, by time. Notices that came before the card linked the pull request are put on it when the link is made. One case cannot be told apart without the commits' order: a review of an older commit that arrives after the new commit's own notices reads as a review of the newest. The authenticated GitHub read (decision C) closes it; until then the card can show an approval of an older commit as current, and GitHub's own merge rules still stand.
- **Derive in the same pass.** The card's pull request pass calls `waits_on()` for each open pull request and gives the card the value of the one that blocks it (order: a person, the author, a reviewer, CI, the queue, nothing; the most recently changed on a tie) with a count of the others. The badge travels with the card in the board payload; no request per card, so a virtualized column needs nothing extra.
- **Cost.** No GitHub call, no credential. One pass over the board's pull request rows, which the store already makes. The payload gains a handful of short fields per open pull request; the build measures the board payload before and after, with a large board, and puts both in its PR.
- **Unmatched pull requests** (the Kanban's existing list) show the same badge when the flow knows them.

## 4. The card

- **One line at most, only for a card with an open pull request.** On the pull request line the existing state pill stays and gains: one **waits-on badge** (the author, a reviewer, CI, the merge queue with its position when known, a person, or ready to merge) and small chips: review verdict (approved, changes requested, stale when the head moved) and checks (passing, failing, running). A card with several pull requests shows the one that blocks it and "+n".
- **Never a guess.** A pull request the rule cannot place shows no badge rather than a wrong one; an ambiguous join shows state only.
- **Narrow and zoom.** Below the card's narrow width the chips collapse to icons (the words move to the tooltip and the drawer); the badge keeps its word. Heights and text follow the board's zoom as the card's other lines do. A card's height does not depend on the data arriving late: the line is reserved once the card has an open pull request.
- **Private repositories** show states only, as today; verdict and checks chips appear only where notices arrive.
- **A board with no GitHub link, or on another machine.** No pull request, no line. A board on another machine shows what that machine's server derived and sent with its cards; the notices exist only for agents on this computer and its LAN peers, so a remote card may show state only.

## 5. The drawer

Per pull request: the verdict of each reviewer kind and the head it was given on, checks (failing first), merge state and why blocked, age and last change. Findings by priority (counts, expandable), per-person reviewers and the required-check list are part of the later authenticated read, not this build; the drawer says "details need a GitHub connection" where they would be.

## 6. Needs you, filters and search

- **A person.** `person` today means a card whose task waits in Review to be accepted, which is already the Review column. The card gets the "waits on a person" badge there; whether it is also shown in Needs you is choice A. A card waiting on a person for any other reason goes to Needs you with the reason.
- **The Reviews groupings** become Kanban filters: a "waits on" filter (anyone, the author, a reviewer, CI, the queue, a person); by repository is the existing lanes; by agent is the holder on the card and the existing search. **Search** also finds a card by pull request number or branch.
- Dropped with the pane: the list of repositories the person picks and the "No task" group (the unmatched list covers it), until an authenticated read exists.

## 7. What is removed, and what keeps working

- **Removed:** the Reviews pane view and its store, the pane registration, the widget entry, the palette and menu entries, the one-per-window listing, and the Kanban drawer's "Open PRs" button's call into the pane (it opens the card's drawer instead).
- **Redirects:** a saved layout, pin, link or widget that names Reviews opens the Kanban (choice B), a deep link to Reviews resolves to the Kanban with the filter, and a request from an agent's pane tool for the Reviews view is accepted and opens the Kanban; none is an error. A `reviews:task` value opens that card.
- **Migration, tested** with fixtures: a saved layout with a Reviews pane in a tab (the pane becomes a Kanban pane filtered to cards with open pull requests), a pin, a link, a widget id, a deep link, and an agent's pane request. The layout migration runs once on load and is idempotent.
- **The route stays, thin.** `GET /api/v1/reviews` is answered from the same persisted rows and the same function, so the two never disagree, with the agent query kept (`?agent=<me>&waits_on=author`), the same audience, and the same visibility rule (an agent sees only the boards its task list shows it). The pane's own code, store and tests go.
- **Kept as it is:** the provider interface design, the flow events, and the rule's table test.

## 8. The Swarm's task line

The Swarm's task line may reuse the card's two fields (the badge value and the blocking pull request); it derives nothing of its own.

## 9. The Reviews spec's parts

R1 (the pane and the route): the pane is removed, the route stays thin (section 7). R2 (an authenticated read: findings counts, per-reviewer detail, queue position, required checks): kept as a later step on top of this one, with its token source decided by the person who owns credentials before any build; this build needs none. R3 (groupings, filters, search): the Kanban filters of section 6 (the repository picker is dropped). R4 (a second code host) and R5 (actions): dropped for now; the provider interface stays as the design R2 builds on. The Reviews spec's status line is updated to say so and its join key corrected.

## 10. Decisions

- **A. Waiting on a person.** A card waiting for the operator's accept stays in the Review column with the badge (Review is where such cards already are). Not also copied into Needs you.
- **B. A saved Reviews pane, pin or link.** It opens the Kanban filtered to cards with open pull requests (what the pane showed).
- **C. The authenticated GitHub read** (findings by priority, per-reviewer detail, queue position, required checks): later, after the fold, with its token source decided first. This build needs no credential.

## 11. Build split (small PRs, each with the look check on macOS and Linux where it changes how something looks)

1. **srv:** persist verdicts, checks and merge state on arrival with the repository id rule; derive the card's badge and blocking pull request in the pass; the thin route over the same rows. Tests: the join fixtures (one candidate, ambiguous, restart persistence), the rule's existing table, the route's grant test, the payload measurement. This is the store's pending "review and checks on the card" step.
2. **The card and the drawer:** badge and chips, drawer section, the filter and search, Needs you; narrow and zoom, light and dark.
3. **Removal and migration:** the pane, registrations, redirects, the deep-link and agent-tool entries, the migration with its fixtures, the Reviews spec's status line, docs for the current release only.
