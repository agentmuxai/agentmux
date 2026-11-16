# Spec: a Reviews pane, every task's pull request and its review state in one viewer

**Status:** active — R1 (the pane and `GET /api/v1/reviews`, no GitHub call) shipped in PR #7435; R2 to R5 not started. Verified 2026-11-15. Approved by the operator 2026-11-15 ("reviews spec looks good, proceed"). Task Area54 #2581 (spec), #2582 (build). Written 2026-11-15; read from origin/main 09cb7140026, origin/main was 510907e5239 when it was finished (the files cited were checked there). Owner of the spec: Lazo (events, merge queue). Pane lane: AgentX (agreed 2026-11-15, section 7); Kanban links: Naki-2 (section 4); pane conventions and the code review of the pane: Camper.

## 1. What the operator asked for

In their words (2026-11-15): "lets also spec a new pane for reviews, essentially we want it general for use on other vcs, but applicable now to our github review system. in that case the tasks PRs would be organized into a pane, you can keep the data remote, the pane is just a viewer, lets also do a research for best practices write spec to file".

Read as: a new pane that lists pull requests and where each stands in review, organized around the Kanban tasks they belong to. The data stays on the remote (GitHub now); the pane keeps no copy of its own beyond a cache, and in its first form changes nothing there. Its design is not tied to GitHub: a provider interface, so another host can be added later. This spec writes up the design and the research behind it.

## 2. In one paragraph

A Reviews pane (its own pane type, opened from the pane menu and the command palette like Tower, History and Drone) shows a list of pull requests grouped by the Kanban task they belong to, with a **Needs a person** group on top, and a detail side for the selected one. A row says who the PR is from (agent and host), what review it has (a review bot's verdict, a second reviewer's findings by priority, stale when the head moved), what its checks are (failing first), where it is in the merge queue, and, most useful, **what it waits on** (the author, a reviewer, CI, the queue, a person). The first build needs **no GitHub call and no new credential**: it joins the Kanban cards (title, agent, repository, branch, PR address, merged) with the review, checks and merge states AgentMux already receives (`GET /api/v1/pr-flow`, shipped in #7309 and #7313). A second phase adds an authenticated, cached, budgeted read of GitHub with the person's own token, for titles of PRs that have no task, findings by priority, per-reviewer state and queue position. The same data is a route (`GET /api/v1/reviews`) so agents and container agents can ask "which of my PRs wait on me" without opening the pane.

## 3. What the app already has (the survey, read from origin/main 09cb7140026)

- **The Kanban card** holds one PR: `card.pr_url` (a single string, set by `Tasks update links.pr`, or at hand-in when the result names exactly one PR and GitHub says its head is the agent's branch), `repo`, `branch`, and once srv has seen the merge `card.pr` = `{merged_at, released_in, released_at}` (`crates/srv/src/backend/storage/work_board.rs`, `kanban/prs.rs`). A task with a spec PR and a build PR holds only the latest. The board never reads the PR body's agent tag.
- **srv holds no GitHub credential.** `kanban/prs.rs` reads GitHub's public API without one (60 calls an hour per address, shared with everything else on the computer; it spends at most 20 an hour and stops at GitHub's reset). A private repository stays as it was.
- **Review, checks and merge states already arrive.** Every genuine review, checks or merge notice for a PR of a local agent carries, from 2026-11-14, a small object (kind, repository id, PR number, a state token, who, the head). srv applies it only when the notice is verified and fresh, and keeps the last 100 of each kind plus the time of the last notice from the review bot. `GET /api/v1/pr-flow` returns `{review, checks, merge, bot: {last_notice_at_ms}}` and three events (`review-flow`, `checks-flow`, `merge-flow`) carry them live. They hold ids, numbers, states and times only: no title, no review text. (`SPEC_REVIEW_AND_CHECKS_FLOW_EVENTS_2026_11_14.md`.) Atlas reads them today.
- **The PR title and body carry the author.** Every agent PR's title starts `<Name>@<host>: ` and its body ends with a hidden tag naming the agent's id (`SPEC_MUXBUS_GITHUB_REVIEW_NOTIFICATIONS_2026_06_20.md` section 5). So a PR can be tied to an agent without asking anyone.
- **What a person sees on a public PR** is what the pane can describe without any internals: a review by a bot account with a verdict (approved or changes requested) and a list of findings, each marked by priority `[P0]`, `[P1]`, `[P2]`; a second reviewer's status check (`Codex review`); the required checks (`check`, `CI required`, `changeset`, the PR-title check); and GitHub's merge queue.
- **A new pane type** follows Atlas's registration, in the lane AgentX confirmed: its own view id, a per-pane theme (theme tokens only), a pane-level Suspense boundary, thin-pane rules.
- **AgentMux Web and container agents** use the same srv routes: nothing here may be a host-only route (`SPEC_HEADLESS_SERVER_PACKAGE_AND_LOCAL_WEB_UI_2026_11_12.md`).

## 4. What it shows, per pull request

| Field | Source | Given by GitHub, or derived |
|---|---|---|
| Title, number, repository, branch, head commit | Kanban card (`pr_url`, `repo`, `branch`); phase 2: the pull request object | given (the card's title is the task's title, the PR's own title comes in phase 2) |
| Author agent and host | the `<Name>@<host>: ` title prefix; the hidden agent tag in the body | derived from the title; the tag needs the body (phase 2) |
| The task it belongs to | the card that holds the PR; the branch; the agent tag (section 5) | derived |
| Draft, open, closed, merged | `merge` flow state; phase 2 `state`, `isDraft`, `mergedAt` | given |
| Review state per reviewer: approved, changes requested, commented, none, **stale because the head moved** | `review` flow state and its head; phase 2 the latest review of each reviewer (`latestReviews`, `reviewDecision`) | given for the verdicts; **stale is derived**: a verdict whose head differs from the PR's head |
| Findings by priority (`P0`, `P1`, `P2` counts, expand for the text) | phase 2, the review bot's review body | **derived**: the bot's findings are a list of lines `- [P1] path:line - text`; the count is parsed from it |
| Checks: required and not, failing first | `checks` flow state; phase 2 `statusCheckRollup` and the branch's required contexts | given for results; **required is derived** from the branch protection or ruleset when readable, else from a configured list |
| Merge state: blocked and why, queued with position, merged, closed | `merge` flow state; phase 2 `mergeStateStatus`, `mergeQueueEntry` (position, state) | given; "why blocked" derived from which required item is missing |
| Age and time since the last change | the card's times; phase 2 `createdAt`, `updatedAt`, last commit and last review times | given |
| **What it waits on** | derived (section 4.1) | **derived** |

### 4.1 What it waits on

One value per PR, derived the way Gerrit's attention set and Phabricator's "Action Required / Waiting on Others" do (section 9), from the review, checks and merge states:

| Waits on | When |
|---|---|
| **a person** | the review bot approved but a human decision is open (a protected-path change flagged for a person, a task awaiting the operator's accept), or a reviewer who is a person was requested |
| **the author** (the agent) | changes requested on the current head; a check failed on the current head; a review is stale and the head did not move since the verdict (nothing new to re-review) |
| **a reviewer** | no verdict on the current head, or the verdict is stale because the head moved; the second reviewer's status is pending |
| **CI** | a required check is pending or running |
| **the queue** | the PR is in the merge queue (position n) |
| **nothing** | merged, closed, or approved and green and not yet queued (then: "ready to merge" is shown as the author's step) |

Bots never count as the person who acts (Gerrit does the same: service users are never put in the attention set). The rule is data: it lives in one function in srv with a table-driven test, so a change of the review system changes one place. A PR the rule cannot place shows "unknown", never a guess.

## 5. How it is organized

- **The default: grouped by Kanban task**, each group headed by the task's number, title and state (waiting, held, review, merged, released), the PRs of that task under it, most recently changed first. This is the operator's line: "the tasks PRs would be organized into a pane".
- **The first group is always "Needs a person"**: every PR whose waits-on is a person, across tasks, oldest first (Gerrit's "Your turn" section, Graphite's "Needs your review"). When empty it says so in one line and collapses.
- **Other groupings**, one switch in the pane header: by agent, by repository, by what it waits on (the board of states). A filter row (agent, repository, waits-on, state) and a search over title, number, agent, branch. The filters and the grouping are the pane's own state and persist with the pane; nothing is written to the remote.
- **A PR with no task** (an agent's PR without a card, a dependency bot's PR) appears in a group "No task", per repository, once phase 2 can list a repository's open PRs (section 6.4); in phase 1 it appears only if an agent's notice mentioned it.
- **A task with several PRs** (a spec PR and a build PR, a follow-up): see 6.3.

## 6. Where the data comes from, with nothing copied

### 6.1 Phase 1: the Kanban and the flow events, no GitHub call

srv builds the list on demand by joining two things it already holds in memory or in its own store: the Kanban cards that have a `pr_url` (title, task state, agent, repository, branch, merged times) and the recent flow entries of `pr-flow` (state tokens, who, head, times). The flow entries name a repository only by its numeric id (never its name), so the join is by the card's holder and the PR number; one repository id learned from a match with a single candidate settles a later clash of two cards of one agent that hold PRs with the same number, and an entry no card claims is a row with no task and no address (the "No task" group). (Found while building R1; the first version of this section said the key was the repository name.) Cost: **0 GitHub calls**, no credential, the same on a container agent and on AgentMux Web. What it cannot know: a PR that no local card and no notice names; the PR's own title; the findings text. The flow entries are bounded (100 per kind) and hold only PRs of agents on this computer and its LAN peers whose notices arrive here, so the list is the fleet's PRs the person is working with, which is what the pane is for.

### 6.2 Phase 2: an authenticated read of GitHub, cached and budgeted

Needed for titles of PRs without a task, findings by priority, per-reviewer state, queue position and required checks. Rules:

1. **The person's own token, never a new credential and never a shared service token.** Today srv has none. The pane asks for one through what AgentMux has for credentials: a GitHub token the person stores in the Vault (the pane's empty state says so and links to the Vault entry), or the Connect with GitHub sign-in when it lands (decision D3). The token is read in srv, never sent to the frontend, never given to an agent. A token that many agents and tools share is **not used**: its hourly limit is shared by everyone who uses it, and a burst by one tool would empty the pane's budget; a person's own token has its own limit.
2. **One GraphQL query per refresh, for all PRs.** GraphQL fetches up to 100 pull requests in one request by id (`nodes(ids: [...])`), each with `reviewDecision`, `latestReviews(first: 10)`, `reviews(last: 1)` for the bot's body, `commits(last: 1) { statusCheckRollup }`, `mergeStateStatus`, `mergeQueueEntry { position state }`, `isDraft`, `updatedAt`. GitHub's documented cost: the sum of the requests each connection would need at its `first`/`last`, divided by 100, minimum 1 point (primary limit 5,000 points an hour for a user, secondary 2,000 points a minute and 100 concurrent requests; docs.github.com, rate limits and node limits for the GraphQL API, read 2026-11-15). For 50 PRs that is about 5 points a query at the sizes above (to be measured in the build with the query's `rateLimit { cost }` field, which GitHub returns for free).
3. **A stated budget.** The pane's budget is **at most 300 points an hour** per srv (6% of a user's hour), spent as: a refresh when the pane is visible and focused at most every 60 seconds only while a PR it shows is not final (final = merged or closed: never asked again); a refresh of the one PR named by a `review-flow`, `checks-flow` or `merge-flow` event (the events already tell us when something changed, so polling is the fallback, not the engine); a background refresh every 10 minutes when the pane is closed but the route is read. The limits are read from the `rateLimit` field and the `x-ratelimit-*` headers of every answer; when remaining falls under 20% of the hour, the refresh interval doubles; when it is out, the pane shows **"Rate limited, retry at 14:32"** (the reset time from the header) and keeps the last cached data with its age.
4. **Cache.** In memory, per PR, with the time it was read, and written to the pane's own small file so a restart shows the last list at once with "as of ..."; entries older than 24 hours are dropped. Staleness is shown, never hidden: each row's tooltip and the pane header say how old the data is. Merged and closed PRs are cached without limit of age but not refreshed.
5. **Nothing is read per open or per pane.** The cache and the refresh belong to srv, one per srv, whatever number of panes or agents read the route. A second pane, a container agent and AgentMux Web read the same cache.
6. **Failure states are stated**: no token ("Connect GitHub to see review details"), token without access to a repository ("No access to owner/repo: showing what notices say"), rate limited (above), GitHub unreachable (last data, its age).

### 6.3 Linking a PR to a task (section 4 of the card)

In this order, first match wins: (1) the card's `pr_url`; (2) the card's `branch` equal to the PR's head branch in the same repository; (3) the hidden agent tag in the PR body plus the task the agent holds or last held; (4) none: "No task". Today a card holds one PR (Naki-2, 2026-11-15), so a task whose spec PR and build PR both exist shows only the latest by (1); (2) and (3) bring back the other one in phase 2 when its branch or agent matches. A task that should list several PRs reliably needs a new field, a list of PRs per task, which is srv schema work in the Kanban area: **decision D5**, with Naki-2 as its spec owner; the first build does not depend on it. The reverse link: the Kanban card's "Open PRs" button opens the pane at its task (section 8.3).

### 6.4 Which repositories phase 2 lists

A PR is shown if a task or a notice names it (phase 1's set). To also list PRs nobody named (a dependency bot's PR, a PR by a person), the person picks repositories in the pane's settings (a short list, default: the repositories of the cards on the board); phase 2 then reads each one's open PRs with one extra connection per repository inside the same query. The default is off for repositories not on the board.

### 6.5 What a container agent and AgentMux Web get

The same pane, the same route, the same cache. No host-only route. The route filters its answer to what the caller may see: an agent that has no grant for a repository does not receive that repository's PRs (the route applies the same repository grants the Browser and the GitHub tools use), and a private repository's PR never appears in an answer to a container agent that lacks the repository. The person's own pane shows everything the person's token can read.

## 7. The provider interface, so it is general

The pane and the route speak one small model; a provider maps its host to it. In TypeScript terms:

```ts
interface ReviewProvider {
  id: string;                                   // "github", "gitlab", ...
  listChangeRequests(scope: Scope): Promise<ChangeRequest[]>;     // open ones, plus those named by tasks
  getChangeRequest(ref: Ref): Promise<ChangeRequest>;              // one, for event-driven refresh
  webUrl(ref: Ref): string;                                        // where "open" goes
}
interface ChangeRequest {
  ref: Ref; title: string; authorLogin: string; state: "open" | "draft" | "merged" | "closed";
  branch: string; head: string; createdAt: number; updatedAt: number;
  reviews: Review[];          // per reviewer, with the head each refers to
  checks: Check[];            // name, required, state: pending | pass | fail | skipped, url
  merge: { state: "ready" | "blocked" | "queued" | "merged"; blockedBy: string[]; queuePosition?: number };
  labels: string[];
}
interface Review { reviewer: string; kind: "person" | "bot"; verdict: "approved" | "changes" | "comment" | "none"; head: string; findings?: Counts }
```

The waits-on rule (section 4.1) runs on this model, not on a host's own words. Nothing in the pane is named for GitHub.

| Host | How it maps | Where its model differs |
|---|---|---|
| **GitHub** (full) | pull requests: `reviewDecision`, `latestReviews`, `statusCheckRollup`, `mergeStateStatus`, `mergeQueueEntry`; reviews carry the commit they were made on, so stale is derived | merge queue is GitHub's; required checks from branch protection or rulesets |
| **GitLab** merge requests | `approvals`, `reviewers`, `head_pipeline`, `detailed_merge_status` (and merge trains for the queue) | approval rules instead of a single verdict; GitLab's own "attention requests" were shipped behind a flag in 14.10 and its epic says the team decided not to launch them (checked 2026-11-15; the current state is unconfirmed), so waits-on is always derived, never read |
| **Gitea / Forgejo** pull requests | reviews with states (approved, request changes, comment, pending), commit statuses | no review decision field; derive it from the latest review of each requested reviewer; no merge queue (a merge scheduling flag only) |
| **Bitbucket** pull requests | participants with `approved` and `needs_work`, build statuses | per-participant booleans; no stale marker, derived from the commit |
| **Gerrit** changes | patch sets and labels: a vote `Code-Review +2/-2` and `Verified` per patch set instead of approve or request changes; the **attention set** gives waits-on directly | a change's identity is the Change-Id, and a new patch set replaces the head; the model's `head` is the patch set; waits-on may come from the host |
| **Local repositories with no host** | out: nothing to ask. A later phase could read a local branch's own review notes | |

Each provider ships with a recorded-response contract test, so a host's model change shows up in CI.

## 8. The pane itself

### 8.1 Layout

A list with a detail side (AgentX, 2026-11-15). Grouped as in section 5. **Below about 440 px the detail replaces the list with a back button**: the thin-pane rule is no sideways scroll at 280 to 440 px (the thin-pane fixes #7080, #7101, #7110 and the narrow Swarm row #7316 are the model). Sketch at about 900 px:

```
Reviews                          [by task v] [repo v] [waits on v] [search...]      as of 14:07
-------------------------------------------------------------------------------------------
v Needs a person (2)                      | #7368  Camper@narko: Editor: ...
  #7371 Drone: cancel a run   P1 x1       |  task 47601  Spec: ...        waits on: a person
  #7365 Release 0.60.29      checks ok    |  agentmux  camper/editor-hex   head 9f3a1c2
v Task 2575  Build: release tag gate      |  Review   bot: approved       (head 9f3a1c2)
  #7363 release-gate  queued 2 of 5       |           Codex: success
  #778  tag job       merged 2h           |  Checks   required 5/5 green   1 optional failing
v Task 2569  Allow rule    merged         |  Merge    queued, position 2
...                                       |  Findings P0 0  P1 0  P2 2   [show]
                                          |  [Open on GitHub]  [Open in browser pane]
```

At narrow width a row is two lines: `#7371 title ...` and `waits on a person | P1 x1 | 2h`; the PR and the detail are one tap apart.

### 8.2 Row content

Wide: PR number and title (task title if the PR's own is unknown), agent chip, a verdict chip (approved, changes requested, stale), a checks chip (green, failing first with the count), the merge chip, the waits-on chip, age. Narrow: number, title (two lines at most), waits-on chip and the single worst chip. Chips use theme tokens only (per-pane theme awareness, #47695): no hard-coded colour; a state is also a word, never colour alone. Findings are shown as counts by priority; **expanding shows the review bot's text** (phase 2). Colour semantics follow the app's existing status chips.

### 8.3 Opening and linking

- Open the PR, its diff and the review: the PR's address opens in a Browser pane or the external browser, as other PR links in the app do. Nothing else happens in the first build.
- The pane's view id is `reviews`; its meta key `reviews:task` (a task number) opens it filtered to a task: the Kanban card's "Open PRs" button calls `openOrFocusPaneByView("reviews", { meta: { view: "reviews", "reviews:task": id } }, { "reviews:task": id })`, the same shape as `openKanbanTask` (Naki-2 adds the button once the view id and key exist). A key `reviews:pr` (`owner/repo#n`, as a `reviews?pr=<repo>#<n>` open target) opens it at one PR. Atlas wants nothing from the pane today and has no PR nodes, so no Atlas link is built; the key is for later links. An `agentmux://reviews/<n>` link kind is a small addition to the shared link layer (`link-target.ts`, `link-destinations.ts`) in the build.
- **No window modal.** Any question the pane asks (clear the cache, remove a token) is a panel inside the pane, per the no-window-modals rule.

### 8.4 Empty and error states

No tasks with PRs ("No pull requests on the board yet"); no connection (phase 2: "Connect GitHub to see review details", with the Vault link, and the phase-1 list still shown); no permission on a repository; rate limited with a retry time and the cache's age; GitHub unreachable; a PR the rule cannot place (waits-on "unknown"). Each is one line with the one action that fixes it.

### 8.5 An agent can read it

`GET /api/v1/reviews` (and the MuxAPI `Pane` read of the pane) returns the same list: `[{repo, number, title, task, agent, host, state, review: {...}, checks: {...}, merge: {...}, waits_on: "person|author|reviewer|ci|queue|none|unknown", waits_on_agent, updated_at, as_of}]`, filterable by `agent`, `waits_on`, `task`, `repo`. "Which of my PRs wait on me" is `?agent=<me>&waits_on=author`. The route declares its audience (agents and container agents allowed, repository grants applied), per the route-audience rule, and is read-only.

## 9. Viewer first, actions later

The first build opens the PR, the diff and a review in the browser pane or the editor and does nothing else. Actions that could come later, each its own decision, each needing a write permission on the token (so a separate, explicit grant, never silently widened):

| Action | Needs |
|---|---|
| Re-request a review | write on pull requests; for the review bot, a comment trigger (`review again`) the person's token posts as the person |
| Merge / add to the merge queue / take out of it | write, and the merge rules of the repository; a confirm panel in the pane |
| Comment | write; an editor in the pane (a larger piece of UI) |
| Send the findings to the author agent | no GitHub write: a message to the agent through the existing agent message route, with the findings text; a natural first action |
| Mark a PR as watched (a pin) | the pane's own state only |

## 10. Research (read 2026-11-15; links are the pages read)

What review dashboards do well, and what applies to a fleet whose authors are agents and whose first reviewer is a bot:

- **Gerrit's attention set** ([user-attention-set](https://gerrit-review.googlesource.com/Documentation/user-attention-set.html), available since Gerrit 3.3): every change has a set of users expected to act next, shown as an arrow before the name; reviewers are added when added to a change, the owner when a reviewer replies; the user who replies is removed; everyone is removed when the change is submitted, abandoned or set to work in progress; service users (bots) are never added, and a bot's reply adds only the owner, and only for a negative vote. The dashboard puts **"Your turn"** at the top with a **Waiting** column. The page admits the automatic rules are "not super smart", so the owner can adjust the set by hand, and an "Auto re-add owner" job re-adds the owner to inactive open changes so they are not forgotten. *Applies:* the "Needs a person" group and the waits-on column copy this; bots never count as the actor; staleness gets its own re-add (an old PR returns to the top).
- **Phabricator Differential** ([Differential article](https://secure.phabricator.com/book/phabricator/article/differential/)): the home screen is two groups, **Action Required** (to review, to revise, to push) and **Waiting on Others**. *Applies:* the two-way split is the cheapest honest model of "whose turn"; the pane's group order follows it. (The page does not say whether the project is maintained.)
- **Graphite's PR inbox** ([use-pr-inbox](https://graphite.com/docs/use-pr-inbox)): six default sections: Needs your review, Approved, Returned to you, Merging and recently merged, Drafts, Waiting for review; sections can be edited, reordered, given a custom filter and shared; fuzzy search over title, description and author. *Applies:* sections as saved filters; "Merging and recently merged" as a place for the queue and the last few finished PRs, so a person sees the result of what they waited on.
- **GitHub's own pull request lists and merge queue** ([search qualifiers](https://docs.github.com/en/search-github/searching-on-github/searching-issues-and-pull-requests), [merge queue](https://docs.github.com/en/pull-requests/collaborating-with-pull-requests/incorporating-changes-from-a-pull-request/merging-a-pull-request-with-a-merge-queue)): the inbox is a saved search: `review-requested:`, `reviewed-by:`, `involves:`, `review:none|required|approved|changes_requested`, `status:pending|success|failure`, `draft:`, `is:queued`, `updated:`. The queue view lists the queued PRs and marks yours; the CLI cannot see or remove queue entries. *Applies:* every group and filter of the pane maps to a search qualifier or a GraphQL field, so any host with a search can serve it; the queue is read by GraphQL, not the CLI.
- **GitLab merge requests** ([epic on attention requests](https://gitlab.com/groups/gitlab-org/-/epics/5331)): reviewer and assignee statuses did not say whether action was needed; "attention requests" were built behind a flag in 14.10 and, per the epic, not launched; the docs for it were later removed. *Applies:* whether the host gives it or not, waits-on is derived by the pane, so a host without the feature behaves the same.
- **IDE tool windows**: VS Code's GitHub Pull Requests extension ([docs](https://code.visualstudio.com/docs/sourcecontrol/github), page dated 2026-11-11) drives its list from user queries written in GitHub search syntax (`githubPullRequests.queries`) and opens a PR's description in the editor, with "review mode" on checkout; JetBrains' Pull Requests window ([IntelliJ IDEA 2026.3 help](https://www.jetbrains.com/help/idea/work-with-github-pull-requests.html), dated 2026-11-04) filters by state, author, label, assignee and review status, sorts by activity, marks unseen PRs with a blue dot and shows a white dot on the window icon when something needs attention, and keeps your comments pending until you submit. *Applies:* "unseen since I last looked" is worth a marker; the list is configured by saved queries; a viewer that opens the review in a tool the person already uses is normal.
- **Terminal dashboards**, gh-dash ([repository](https://github.com/dlvhdr/gh-dash)): user-defined sections by filter, per repository, in one YAML config, vim-style keys, custom actions. *Applies:* sections as data; keyboard first.
- **Reviewable** ([docs](https://docs.reviewable.io/)): a reviews dashboard with review state, filters and "Waiting on" counters, and a discussion model with dispositions and unreplied discussions. Only the table of contents was read (the section pages were not), so nothing is claimed from it beyond the existence of a Waiting on counter and unreplied-discussion tracking.
- **Not read** (named in the request): Linear's and JetBrains Space's review views, GitLab's merge request list pages. Linear's pull request links and JetBrains Space's code review are not needed for the design; GitLab's list is covered through the epic above. If the operator wants them, it is a further hour of reading, not a change of design.

What the research says to build:

1. **Attention first.** Every good dashboard puts "your turn" first and shows how long the other side has waited (Gerrit's Waiting column). Staleness has its own rule: Gerrit re-adds the owner to inactive changes; the pane's age chip and a "stale after N days" marker serve the same purpose.
2. **Derive whose turn it is, from data, in one place.** Hosts disagree on whether they give it; a rule table that every provider feeds is the one thing that makes the pane general.
3. **Sections are filters.** Graphite, gh-dash, VS Code and GitHub itself all configure the list as saved queries. The pane's groups are named filters; a person can add one.
4. **Bots are not actors.** With a bot as first reviewer and an agent as author, the interesting question is "which PRs need a *person*" and "which need *their agent*". The bot's verdict is state, its findings are the thing to show (counts by priority, expandable), and a new push makes its verdict stale, not wrong.
5. **Show the result of waiting.** A short "Merging and recently merged" section (Graphite) answers "did it land" without a trip to GitHub.

## 11. Phases (each a mergeable build with tests)

| Phase | Content | Needs | Leaves out |
|---|---|---|---|
| **R1** | The pane and `GET /api/v1/reviews`, read-only, **no GitHub call**: the Kanban cards joined to the flow states; groups "Needs a person" and by task; the waits-on rule with its table test; thin-pane layout; empty states; "Open PRs" button on the card (Naki-2); `reviews` view id and `reviews:task` key | nothing new | titles of PRs without a task, findings text, per-reviewer state, queue position |
| **R2** | The authenticated provider for GitHub: one GraphQL query, a token from the Vault, the cache file, the budget and the rate-limited state; findings counts and expand; per-reviewer chips; queue position; required checks | decision D3 (token source) | PRs of repositories nobody named |
| **R3** | Other groupings (agent, repository, waits-on), filters, search; repositories the person picks (6.4); "No task" group; the unseen marker | R2 | other hosts |
| **R4** | A second provider (GitLab merge requests) behind the same interface, to prove the interface; contract tests from recorded responses | decision D6 | actions |
| **R5** | Actions: send findings to the author agent first (no write), then the others (section 9) | a write grant | |

Tests: the waits-on rule is table-driven (every state combination in section 4.1 and the stale case); the join has fixtures for a PR with and without a task, a task with a PR no notice names, and a notice with no card; the budget has a fake clock test (the interval doubles under 20%, stops at zero, resumes at reset); the route has a grant test (a container agent without the repository does not receive it); the pane has the thin-pane test at 280 and 440 px and a theme-token lint. Look checks on macOS and Linux before the merge of R1.

## 12. Risks and notes

- **R1's list is only as complete as the notices.** A PR no agent of this computer or its LAN peers was notified about does not appear until R2 and R3. This is stated in the pane ("showing pull requests your agents were notified about").
- **The findings parse depends on the review bot's format** (`- [P1] path:line - text`). That format is part of what the bot posts on public PRs and has been stable; the parser fails soft (counts "unknown", shows the text).
- **The shared hourly limit.** The design never uses a shared token's quota and caps its own spend (6.2). The first build measures the real cost with GitHub's `rateLimit { cost }` field before R2 ships the 300-point budget.
- **Private repositories.** The person's pane shows what their token can read; the route never gives a private PR to a caller without the repository (6.5).
- **A task has one PR today.** The pane works with that (6.3); the list-of-PRs field is optional and separate (D5).
- **A verdict with no head can't be told stale.** A notice that carries no commit gives a verdict with no head, so a push after it does not mark it stale until the pull request ends or a newer verdict arrives; until the new checks come in, an old approval of that kind reads "ready to merge". The review bot's notices carry the head, so this is the exception, and R2 reads the real head.
- **A busy board.** The list reads each board the caller sees on its own, newest 500 tasks (open ones and those finished in the last 7 days), so one board's tasks never push another's out. A single board with more than 500 such tasks loses its oldest ones from the list until R2 asks only for tasks that name a pull request.
- **A finished task.** A task that is done, cancelled or failed whose pull request no notice saw end shows as "The task is finished", waiting on no one.
- **"A person".** `person` is the one value for a decision that is not an agent's turn; today it is a task waiting in Review to be accepted, which an agent may also do. The pane says "the task waits in Review to be accepted", not that a human must act.
- **Two sources can disagree** (a flow state older than a refreshed GraphQL read): the newer by time wins and the row shows "as of" for the older source.

## 13. Decisions for the operator

Each with my recommendation.

1. **D1. The pane's name.** "Reviews" (the operator's word). *Recommended:* **Reviews**; the view id is `reviews`.
2. **D2. The default grouping.** By Kanban task, "Needs a person" on top, as the operator's line says and AgentX confirmed. *Recommended as written.*
3. **D3. Where the GitHub token comes from for R2.** (a) a token the person stores in the Vault; (b) the Connect with GitHub sign-in when it reaches the app; (c) neither: R1 only. *Recommended:* build R1 now (no token), R2 with (a) first because it needs nothing new, and move to (b) when it lands; never a shared service token.
4. **D4. Is R1 enough to ship?** R1 is useful (every task's PR, review, checks, merge, waits-on) with no GitHub call, but only for PRs agents were told about. *Recommended:* ship R1 as soon as it is built and judge R2 by use.
5. **D5. Several PRs per task.** Add a list of PRs per task to the Kanban (a schema field, spec by Naki-2), or live with one PR per task plus the branch and agent-tag joins. *Recommended:* live with the joins for R1 and R2; ask Naki-2 for the field only if a spec PR and a build PR on one task are common enough to matter in use.
6. **D6. A second host (R4).** Build GitLab merge requests as the proof of the interface, or leave other hosts until someone needs one. *Recommended:* leave it; keep the interface and its contract test shape, and build a second provider when a repository on another host is real.
7. **D7. PRs nobody named (R3).** List a repository's other open PRs when the person picks it. *Recommended:* yes, off by default.
8. **D8. Actions.** Viewer only for R1 to R4; "send the findings to the author agent" first when actions begin. *Recommended as written.*
9. **D9. Who builds it.** The pane's code is in AgentX's lane and Camper reviews it; the route, the join, the rule and the provider are srv work in my area. *Recommended:* I build R1 end to end (srv half and pane) with Camper reviewing the pane code, since the half that matters (the rule and the join) is the part I know; if the operator prefers a pane-owning seat for the UI, the srv half stays with me and the pane goes to Camper or Agent1 against the route's contract from this spec.
10. **D10. The call budget.** 300 GraphQL points an hour per srv (6% of a user's hour) for R2, doubling the interval under 20% remaining. *Recommended as written*; measure with `rateLimit { cost }` first.

## 14. Where each fact lives (for the build)

- Kanban cards: `crates/srv/src/backend/storage/work_board.rs`, `kanban/prs.rs` (the PR address, merged times; `parse_pr_ref`).
- Flow states: `crates/srv/src/backend/pr_flow.rs`, `server/pr_flow_handler.rs`, the route `GET /api/v1/pr-flow`, events `review-flow`, `checks-flow`, `merge-flow` (`frontend/app/store/mps-events.ts`).
- A new route declares its audience in `crates/srv/src/backend/route_audience.rs` and is listed for the container-agent tests.
- **Registering the pane** (Camper, 2026-11-15, read from main, Atlas is the model), four places: (a) `frontend/app/view/reviews/reviews.tsx` exports `reviewsPaneTab = lazyPaneTab({apiVersion: 1, view, label, title, icon, defaultColor, capabilities, members, load: () => import("./reviews-instance")})` (see `view/atlas/atlas.tsx`); (b) that export is added to the list in `frontend/app/block/block-registry.ts`; (c) a palette entry `open:reviews` in `frontend/app/store/command-registry.ts` (`openPane({ meta: { view } })`); (d) srv's one-per-window view list `crates/srv/src/server/app_api/pane_singleton.rs` and the view-name list in `crates/srv/src/backend/layout_file.rs` (~line 693), so a saved layout and pane-open know the name. Whether a new view needs anything for container agents or AgentMux Web is not yet traced: the build checks it with Manoz or AgentX before R1 is called done.
- **Rows:** the Swarm's list component is the base for a row-heavy pane (the Kanban's is card-shaped); the build confirms in `view/swarm` which component is exported before relying on it.
- The Kanban card's open-pane call: `frontend/app/util/open-kanban-task.ts` is the model; link kinds: `frontend/app/element/link-target.ts`.
