# Spec: a message carries the runtime of the agent that wrote it

**Status:** active — the stamp at append shipped in PR #7946 and the fork mark and re-sign in PR #7973; the read-side attribution and the frontend ship in the PR that carries this file. Task #2676, a follow-up to #7878 and #7902. Verified 2026-11-21.

**Area:** Agent pane

## 0. Requirement

Which files a message may show or open, and from where, is decided by who wrote the message, not by the pane that shows it. A message a container agent wrote reads only that agent's own folders wherever it is shown: in a pane of a host agent, in a fork, in a replay. A message a host agent wrote is as before. History that crosses from one runtime to another is marked as it crosses. A client can only tighten what srv says; nothing in the reading path makes a message looser than its pane.

## 1. What exists

- srv decides by the pane (`agent_media_files::pane_meta`, #7878): a container pane's message paths are found in that pane's shared folders; the frontend passes the pane id and `confined`, and a confined pane with no pane id reads nothing.
- The conversation is the agent's output zone, keyed by the agent definition (`agent:<defId>:current`), stored as raw provider lines. The frontend rebuilds its rows by replay, so a mark has to live on the stored line.
- A stored line can carry `amx`, signed by srv with a key an agent can't read (`transcript/origin_sig.rs`). Everything srv hands to a viewer goes through `vet_line`, which removes the record's `amx` and puts back only what a valid signature covers.

## 2. Design

1. **A writer stamp at append (#7946).** srv adds `amx.writer{runtime, agent}` to each non-delta line an agent's process writes, as the last member (the agent's bytes and key order are kept, which the history readers need), signed for the zone's owner. Whatever `amx` the agent's own line carries is dropped first and never merged. Streaming deltas are never stamped. A line that claims a writer in a field the agent wrote has no valid signature, and `vet_line` removes it.
2. **A fork copies stamps and says where the copy came from (#7973).** The copy re-signs each stamped line for the fork, keeping its length so every offset stays valid; a line nobody stamped stays unstamped. The fork's mark (`fork.json`) carries `runtime` (the parent agent's, from srv's own record of it, a parent srv doesn't know counting as a container), `upto` (how many bytes are the copy) and `sig`. Forks that already existed are sealed once at start, with the less trusted of the parent's and the fork's runtime and the whole transcript as the copy. A mark that was not sealed, or does not verify, covers nothing. A transcript whose length can't be measured is left unsealed, never sealed as empty.
3. **Reading: which runtime a line is shown with** (`transcript/attribution.rs`, on all four read paths). A verified stamp wins. An unverified line in a fork of a host agent whose copied history was a container's (or whose mark does not verify) is shown as `amx.writer.runtime = "container"` with `attributed: true`; a verified line is left alone. A message another agent sent (a jekt or forwarded message) is shown with its sender's runtime when that is a container or the sender is not on srv's record; a service's message is not marked. Nothing here marks a line as the host's, and deltas are never rewritten.
4. **The frontend takes `container` only.** Rows made from a record whose `amx.writer.runtime` is `container` are marked `fromContainer`. In a pane that is not a container's, such a row reads nothing from this computer and asks srv for nothing: it is shown as in a confined pane with no pane to ask (the fail-closed case of #7878). In a confined pane, or for any other row, nothing changes. No other value makes a row looser.
5. **Why no change to the fetch.** srv already refuses a confined pane with no pane id, and a path read for a row that carries no pane id is never fetched; the claim needs no new request field, because the only way a client acts on a container writer is to ask for less.

## 3. Named tests

- Stamped line round trip, own-`amx` dropped, forged host or container claims, tamper, moved to another owner, byte layout, init prefix position, duplicate and escaped `amx` key (`origin_sig`, #7946).
- A fork keeps its parent's stamped lines verifying for the fork and its mark says where the copy came from; a newest-part copy is sealed with the length it holds; a changed or missing mark covers nothing; an existing fork is sealed once; the pass takes each runtime from srv's own record and the less trusted of two; a fork whose transcript can't be measured is not sealed (`fork_history_tests`).
- A line a container wrote with a made-up host stamp shows as the container's in a host fork, and the fork's own stamped line is the host's (`a_line_a_container_wrote_with_a_made_up_host_stamp_shows_as_the_containers_in_a_host_fork`).
- A message a container agent sent is marked, one from a host agent or a service is not; a sender not on record counts as a container; a verified stamp is not touched; a delta is never rewritten (`transcript/attribution`).
- `writtenInContainer` reads only `container`; rows, tool rows and a held jekt keep the mark of the record they came in (`stream-parser.writer.test.ts`); a marked row in a host pane loads nothing, and an unmarked one loads as before (`MarkdownBlock.media.test.tsx`).

## 4. Decisions

D1 stamp with `amx` (not a separate index). D2 refuse with the reason where the stamped agent has no folders to show. Both approved 2026-11-20.

## 5. Not in the spec

Reading a container writer's files from that agent's own folders in a pane that is not the container's (a row from a container parent in a host fork is shown without its media here, never from a host path); changing which folders a container agent shares; export and import of a whole history (its own spec; it must apply 2.2); native-agent media rules.
