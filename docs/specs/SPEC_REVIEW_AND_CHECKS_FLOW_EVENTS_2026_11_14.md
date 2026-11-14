# Spec: review and checks flow, two events and a recent list (for the Atlas review bot and CI nodes)

**Status:** implemented by this change; agreed by Camper as the map's owner on 2026-11-14 (answers in section 9); the spec's approval itself is the operator's,
through a leader. Task narko #47597 (spec) and #47598 (build, after this). Written 2026-11-14 from origin/main 25a8a2ea7. Owner: Lazo
(events, merge queue). Requester: Camper (Atlas, spec `SPEC_ATLAS_AGENT_MAP_2026_11_13.md` section 5.4). The cloud half of this work
is separate work; this spec describes only what srv and the frontend do.

## 1. What the map asked for

Atlas wants the review bot and CI as nodes of their own, with a line to each agent whose PR they are working on, and cannot
draw them: neither has any status in the frontend. Asked for, as ids, numbers, states and times only (no review text, no
log, no title):

1. Review: "review of PR N for agent A: approved, changes requested or commented, at T, for head commit H", and the bot's own
   state as seen from here (when its last notice came; whether cloud messages are paused).
2. CI: "checks of PR N at head H: passing or failing, at T".

## 2. What exists today (read from the code on origin/main)

- A review or CI notice reaches the frontend only as the text of a message delivered to one agent. Nothing keeps "PR N was
  reviewed" or "PR N's checks failed" as state.
- srv pulls those notices from the relay (`muxbus/cloud_subscriber/delivery.rs`, `PendingInj`). A pulled message carries its id,
  sender, text, priority, times, the signature fields and a declared tier. It has **no PR number, no kind and no state**;
  the only place those exist on this side is the text.
- srv already checks the notice's signature and freshness on the way in (`delivery_steps.rs`: `verify`, `reagent_checks`,
  `is_repeat`), so at one point in that loop it knows a notice is genuine, fresh and not a repeat.
- The pattern to follow is `message_flow.rs` and `task_flow.rs` (merged as #7173 and #7192): one event, a short in-memory recent
  list, a `GET /api/v1/...` route listed as `Audience::Everyone`, types exported by ts-rs, the event registered in
  `mps_events.rs` so `gen-mux-events` writes the frontend side.
- "Cloud messages are paused" is already pushed to the frontend as the `muxbus:status` event
  (`delivery_status::status()`, `MuxBusDeliveryStatus`), so the bot's online state needs nothing new.

## 3. The decision: where the data comes from

| | A. srv reads the notice text | B. the relay sends a small `flow` object with each notice (recommended) | C. a feed of its own |
|---|---|---|---|
| New cloud work | none | the cloud side fills, stores and serves it | a new route and a stored per-PR state |
| Breaks when | the notice wording changes (it has changed several times) | never by wording; a field the app does not know is ignored | the feed and the notices disagree |
| States it can show | the ones a notice says | the same, plus the head commit and the repository id | also pending and requested |
| Signed | the text is | the object is not (see 5.3) | no |
| Cost | a parser for a private format in the public app | one optional field per row | polling or a second subscription |

**Recommendation: B.** A puts a parser for a private, often-changed text format into the public app, and every wording change
would silently blank the map. C is the only way to get "pending" and "requested" (no notice is sent for them today), and is not
needed for the first map; section 8 says what it would take. B is small, has no wording dependency, and either side can ship
first (an old srv ignores the field, an old relay does not send it).

## 4. The events

One module, `crates/srv/src/backend/pr_flow.rs`, mirroring `message_flow.rs`.

```
PrRef            { repo_id: number, number: number }          GitHub's own ids; never a name
ReviewFlowEvent  { id: string, at_ms: number, agent: string, pr: PrRef,
                   state: ReviewState, by: "bot" | "codex" | "person", head?: string }
ChecksFlowEvent  { id: string, at_ms: number, agent: string, pr: PrRef, state: ChecksState, head?: string }
MergeFlowEvent   { id: string, at_ms: number, agent: string, pr: PrRef, state: "merged" | string }   // the PR's end
```

**States are open-ended.** The names this version sends are `approved | changes_requested | commented` (review),
`passing | failing` (checks) and `merged` (end); the exported TypeScript types are `Known | (string & {})`, so a state this
version does not name is representable, not an error. srv does not drop a state it does not know: it keeps any well-formed token
(lower-case letters and `_`, 1 to 24 long). The frontend draws a state it does not know as neutral. A new state (`pending`,
`requested`, `closed`; section 8) is therefore a cloud-side change only, with no release ordering between the app's srv and
frontend or between the app and the cloud.

- `id` is the relay message's id, so a redelivery is the same event (the recent list drops a repeated id). `at_ms` is when
  this srv accepted the notice. `agent` is the id of the local agent the notice was delivered to: the PR's own agent, so a line
  needs no mapping and the events only ever name this machine's agents (another machine reports its own).
- `head` is the first 12 hex characters of the commit, when the notice has one. `by` is a coarse reviewer kind, not a login.
- **No field for a title, a branch, a URL, a review body, a log, a login or a repository name**, so none can leak through it. A
  repository is named only by its numeric id; the frontend can resolve a name from the agent's own work facts if it wants one.
- Three events, `review-flow`, `checks-flow` and `merge-flow`, each with its own recent list of 100 (oldest dropped), in memory only.
- `GET /api/v1/pr-flow` answers `{ "review": [...], "checks": [...], "merge": [...], "bot": { "last_notice_at_ms": number | null } }`,
  oldest first. `last_notice_at_ms` is the time of the newest accepted notice of any kind; "cloud paused" is read from
  `muxbus:status` as today. Read-only, `Audience::Everyone` (a container agent may read it).

## 5. How srv fills it

### 5.1 The relay's object

`PendingInj` gains one optional field, `flow`, absent from an older relay and from every row that is not a review, CI or merge notice:

```
flow: { kind: "review" | "checks" | "merge", repo_id, pr, state, by?, head? }
```

It is one object, or a list of up to three when one notice is two results (a "ready to merge" notice is an approval and green checks); each object is judged on its own, and the events of one notice share its id, each in its own list.

### 5.2 Where it is applied

In the delivery loop (`delivery.rs`), after `verify` and `is_repeat` and where the message is accepted (delivered or held), once per
notice. It is applied only when **all** of these hold, and dropped silently (a debug log line, no event) otherwise:

1. the signature verified under a trusted key and the notice is fresh (`reagent_verified == Some(true)` and not expired): a stale
   or forged notice shows nothing;
2. the object parses: `kind` is one of the three names, `state` is a well-formed token (section 4), `repo_id` and `pr` are positive
   whole numbers below 2^53, `by` (review only) is one of three names, `head` is 7 to 40 hex characters (kept to 12);
3. its `pr` equals the number in the signed text's first line (`PR #N`). This is an integrity check, not a parser: it ties the
   unsigned object to the signed message, and a wording change makes it fail closed (no event) instead of drawing a wrong line;
4. the delivery target is one of this srv's agents.

### 5.3 What is and is not trusted

The `flow` object is not part of the signed material (`jekt_sign::signed_material` covers message id, sender, target, time and
text), so it is advisory: a relay that lied could draw a wrong state on the map, and could do nothing else, because the events
drive no action and put no text in front of an agent. Rules 1 and 3 above bound it to genuine notices about the right PR. If the
map ever drives something (a notification, a block), the object must join the signed material first, as a versioned field.

## 6. What the map reads

Atlas needs no new store: the three events arrive through the generated `mps-events` like `message-flow`, and
`GET /api/v1/pr-flow` fills the lines when the pane opens. Drawing is Camper's, not part of the build; as agreed: the newest event
per (PR, kind) is the line's state; a `merge-flow` event ends the PR's lines; a line with no end (a failing check on a still-open PR
stays true) is kept, dimmed, with a longer fade only as the fallback; a state the app does not know is drawn neutral;
`changes_requested` and `failing` read as warnings; `by` marks the review line (a Codex or a person's review is marked, not a
node of its own); the bot node shows `last_notice_at_ms` and the paused state.

## 7. Plan (the build, #47598)

One app PR, then the cloud PRs (any order is safe):

1. `pr_flow.rs`: the types, `event_of` / `remember` / `note` / `recent` as in `message_flow.rs`, the validation of 5.2 as a pure
   function; unit tests for the exact JSON, every rejected case, the repeated id, the bound.
2. `PendingInj.flow`, the call in the delivery loop, and a delivery test: a verified notice with a good object makes one event
   with the target's id; a forged signature, a stale one, a wrong PR number, a malformed state and a non-local target make none; a
   well-formed state the srv does not name is kept.
3. The three event names in `mps.rs` and `mps_events.rs`, `GET /api/v1/pr-flow` with its handler, `route_audience.rs`, `routes.rs`; ts-rs
   exports (state types `Known | (string & {})`); `gen-mux-events` and `gen-rpc-stubs` run with `--check`.
4. The README or help line that lists the event routes, if one exists; a changeset.

Nothing here touches the Atlas pane. Camper starts the nodes when `GET /api/v1/pr-flow` answers on a build with a real relay.

## 8. Out of scope, and what it would take

- **"Checks pending" and "review requested": the planned second step, no app change.** No notice is sent when a check starts or a
  review is requested, so there is nothing to forward. Adding them means the cloud side reports two more GitHub events per push
  (a check suite starting, `review_requested`), once per PR head, and sends the same `flow` object with the state `pending`
  or `requested`; a check runs about 25 minutes, so `pending` is what makes a CI node worth drawing, and a review takes about 30
  seconds, so `requested` matters less. These must reach the map **without a new message in the agent's conversation** (one per
  push would be noise); how such a state is carried is the open question of that step. Because states
  are open-ended (section 4), the app ships none of this: an app that does not know the state draws it neutral, one that does
  draws it.
- **`closed` (a PR closed without merging).** The cloud side sends no notice for it today (it would be a new message to the author).
  It can be a `merge-flow` state like `merged`, with the notice the relay needs to carry it; not in the first build.
- **Ready-to-merge and reminder notices** carry no state of their own.
- **Another machine's events.** Each srv reports what it received. A map of two machines joins by `pr` the way the message flow
  joins by `msg_id`.

## 9. The map owner's answers (Camper, 2026-11-14)

1. The first build is enough as written (review: approved, changes requested, commented; checks: passing, failing); `pending` for
   checks soon after, `requested` less urgent; types kept open-ended so the second step needs no release ordering. Taken: sections 4
   and 8.
2. An explicit end, carried by the merged notice that is already sent: a `merge` kind. Taken: `merge-flow` with `merged` in the first
   build; `closed` is the second step (section 8). A failing check on an open PR is kept, dimmed, until an end arrives.
3. `by` stays; one review node is enough for v1, the line is marked by it. Taken: section 4.

## 10. Risks

- A cloud-side change that stops sending `flow` blanks the map without an error: the bot node shows
  `last_notice_at_ms` going stale, which is the visible sign.
- The first-line `PR #N` check couples the app to the notice's first line. It is the one text dependency and it fails closed:
  a wording change shows as no events, never as a wrong line.

Agent: Lazo@Area54
