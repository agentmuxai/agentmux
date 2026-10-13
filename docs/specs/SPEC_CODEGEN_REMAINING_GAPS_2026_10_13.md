# What codegen still doesn't cover

**Status:** proposed — step 1 (typed history reads) in review; steps 2 and 3 next. Written 2026-10-13, after `SPEC_TYPED_EVENT_PAYLOADS_2026_10_13.md`, `SPEC_TYPED_EVENTS_EVERY_PATH_2026_10_13.md` and `SPEC_TSRS_SERDE_ATTRIBUTE_WARNINGS_2026_10_13.md` shipped. The owner asked for "anything missing".

## The gaps

Every RPC stub, every srv type the frontend reads, and every event srv builds is now generated and gated. Three things still aren't:

| Gap | Where | Today |
|---|---|---|
| 1. Event-history reads | 5 frontend callers of `EventReadHistoryCommand` | each parses the stored events' `data` by hand |
| 2. Events clients publish | `POST /agentmux/wps/publish`, used by `agentmux-bashwrap` (`tool_chunk`, `compaction_started`) and the frontend (`singleton:claim`) | srv relays them as sent; the frontend subscribes untyped |
| 3. Untyped RPC handlers | 25 MuxAPI handlers in `server/app_api` (memory, identity, agent define/io/open, bundles) | `register_handler` with a hand `Deserialize` struct and a `json!` response |

**Left alone:** the frontend subscribes to `userinput`, with a modal, but nothing in srv publishes it yet. srv's `SendUserInputResponse` says "user input routing not yet wired", so this is an unfinished feature, not dead code. It is typed when it is wired.

## Plan

One PR per numbered step.

1. **Typed history reads.**
   - **Helper:** `readMuxEventHistory(event, scope, maxitems)` returns a typed event's stored payloads, typed like `onMuxEvent`'s.
   - **Gate:** `gen-mux-events --check` refuses `EventReadHistoryCommand` with a typed event outside that helper.
   - **Callers:** the four with typed events move to it. `singleton:claim` waits for step 2.
2. **Typed client events.**
   - **Shared payload types:** `tool_chunk`'s and `compaction_started`'s payloads move to `agentmux-common`, so `agentmux-bashwrap` publishes the same Rust types srv declares. `singleton:claim`'s is declared in srv and generated for the frontend, which publishes it.
   - **Declaration:** each goes in a list of client-published events.
   - **Validation:** the HTTP route deserializes a declared event's `data` into its payload type and refuses one that doesn't match. Any other event is still relayed as sent.
   - **Frontend:** subscriptions to them use `onMuxEvent`.
3. **Typed MuxAPI handlers, in three batches:** memory and identity (7) with agent define (1); agent io (5) with agent open (1); bundles (11).
   - **Conversion:** each moves to `register_typed`, with a request struct and a response struct that serializes exactly as its `json!` did, plus a test per response.
   - **Ratchet:** `MAX` goes down with each batch, to 0.
   - **No new stubs:** stubs are generated only for commands in the manifest, so these add schema entries and types, not unused stubs.

## Done when

- **Ratchet:** `check-untyped-rpc-handlers` reports 0 untyped handlers.
- **History reads:** every history read of a typed event goes through the typed helper.
- **Relay:** it forwards a declared client event only if its payload matches.

## Not in scope

- **Relaying undeclared events:** the HTTP route still forwards an undeclared event as sent, for external tools.
