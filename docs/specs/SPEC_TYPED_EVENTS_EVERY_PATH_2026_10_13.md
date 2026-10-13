# Typed events on every path

**Status:** proposed — part 1 (the helpers, the wider ratchet, the handler check, and every site but the LAN events) in review; part 2 (the LAN events, `LanInstance` exported, the ratchet at 0) next. Written 2026-10-13, after `SPEC_TYPED_EVENT_PAYLOADS_2026_10_13.md` shipped (#4791–#4803).
**Follows:** that spec. Its ratchet reached 0, but it counted only two of the ways srv sends an event, so events sent the other ways are still untyped.

## The gap

The ratchet in `scripts/check-untyped-rpc-handlers.mjs` counts two things:
- **Hand-built broker events:** a `MuxEvent { … }` literal.
- **Hand-built frames:** an `eventrecv` frame built by hand.

It misses three more:

1. **Raw WebSocket events.** A `WSEventType { eventtype: "<name>", oref, data }` sent with `EventBus::broadcast_event` (or a lane variant) never touches the broker.
   - **How it reaches the frontend:** `websocket.rs::forward_event` wraps it as an `eventrecv` frame named after `eventtype`. The frontend then subscribes to it like any broker event.
   - **The comment there:** it says such an event is "typed where it was published", which is wrong for these.
2. **Hand-built events in the exempt files.** The ratchet skips `backend/mps.rs` and `backend/eventbus.rs` whole, to exempt the typed helpers. That also exempts the untyped publishes written there.
3. **Handlers still written against the event.** A subscription converted to `onMuxEvent` whose handler is typed `(event: any)` still reads `event.data`. It now receives the payload itself, so the read is silently `undefined`, and `tsc` can't see it. #4799 found three of these with a local script that CI doesn't run.

| Path | Sites | Events |
|---|---|---|
| Raw WebSocket events | 23 | `waveobj:update` (10), `laninstances` (4), `laninstances:error` (2), `laninstances:health` (1), `laninstances:firewall` (3), `lsp:message`, `muxbus:status`, `viewer:paired` |
| Hand-built in `mps.rs` | 2 | `install_progress`, `block:activity` |
| Hand-built in `eventbus.rs` | 1 | `waveobj:batchedupdates` (`broadcast_mux_obj_updates`) |

**Mismatch:** `waveobj:update` is already a typed event (`MuxObjUpdated`). The 10 raw sites send it without that type, so its payload is typed on one path and not the other.

**Not a gap:** the frontend subscribes to `userinput`, but nothing in srv publishes it. Removing that dead subscription is a separate cleanup, not part of this.

## Design

1. **A typed raw event.** `EventBus::typed_ws_event::<E>(&payload, oref) -> WSEventType` builds the frame those sites build by hand, with `E::NAME` and `payload_data(payload)`.
   - **Behaviour unchanged:** each site keeps the `broadcast_event` or lane call it uses today, so the lane, the scope (`oref`) and the wire stay the same.
   - **Scoped helpers:** `publish_install_progress` and `publish_block_activity` move to `publish_event`. `broadcast_mux_obj_updates` sends a declared `MuxObjBatchedUpdates` event.
2. **Declare the events** in `mps_events.rs`, each with a payload struct that serializes as today's `json!`, with a test per struct:
   - `LanInstances`, `LanInstancesError`, `LanInstancesHealth`, `LanInstancesFirewall`;
   - `LspMessage`, `MuxbusStatus`, `ViewerPaired`;
   - `InstallProgress`, `BlockActivity`, `MuxObjBatchedUpdates`.

   Where a payload already has a Rust type (the LAN instance list, the firewall and discoverability states, the MuxBus delivery status), it is exported and its hand-written frontend type is removed. Any other global that duplicates one goes too.
3. **The ratchet counts every path.**
   - **Raw events:** a `WSEventType { … }` literal whose `eventtype` is not `WS_EVENT_RPC` counts as a hand-built event.
   - **No more file exemption:** the typed helpers themselves (`publish_event`, `publish_event_for`, `broadcast_typed`, `typed_frame`, `typed_ws_event`) are marked `// typed-helper: <why>`. They are counted against an exact `MAX_TYPED_HELPERS`, the way relays are counted against `MAX_RELAYS`.
4. **The handler check runs in CI.** `gen-mux-events --check` refuses an `onMuxEvent` handler whose first parameter is typed `any`, `unknown` or `MuxEvent`, or whose body reads `<param>.data`. It finds them with the TypeScript parser, as the comment check does, and has tests.
5. **Frontend.** The subscriptions to these events move to `onMuxEvent`:
   - `global.ts` (MuxObjBatchedUpdates, the four `laninstances*` events, MuxBus status, ViewerPaired);
   - `lsp-client.ts`, `useBlockActivity.ts`, `launch-flow.ts`.

   Their handlers read the typed payload, and their casts and per-field `typeof` checks go. A check stays only where a test requires a malformed payload not to throw.

## Done when

- **Ratchet:** `check-untyped-rpc-handlers` counts 0 hand-built events across all three paths, with 2 relays and the typed helpers counted exactly.
- **Handler check:** `gen-mux-events --check` fails on a handler written against the event, and passes on main.
- **Comment:** `forward_event`'s comment no longer claims every raw event is typed.

## Not in scope

- **The 25 untyped RPC handlers:** they have their own ratchet.
- **Generated types that are themselves wrong:** ts-rs dropping `#[serde(...)]` attributes is `SPEC_TSRS_SERDE_ATTRIBUTE_WARNINGS_2026_10_13.md` (#4802).
- **Event-history reads:** `EventReadHistoryCommand` returns stored events untyped.
