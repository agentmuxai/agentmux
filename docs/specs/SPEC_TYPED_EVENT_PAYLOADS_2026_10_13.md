# Typed event payloads

**Status:** active — step 1 (infrastructure, `controllerstatus`, `sysinfo`) shipped in #4791, step 2 (agent events) in #4792, step 3 (subagents and dispatches) in #4793, step 4 (files and blocks) in #4794; step 5 (the no-payload `*changed` signals) in review; steps 6–7 not started. Verified 2026-10-13.
**Follows:** the RPC stub generation (#4759 to #4777). Every RPC stub and every srv type the frontend reads is generated now. Event payloads are the one part of the srv↔frontend wire that is still untyped.

## The gap

Broker events reach the frontend as `MuxEvent` (`crates/srv/src/backend/mps.rs`). Its envelope is generated (`event`, `scopes`, `sender`, `persist`), but `data` is `any`.

| What | Count |
|---|---|
| Event names the frontend knows (`frontend/app/store/mps-events.ts`) | 81 |
| `muxEventSubscribe` sites | 106 |
| `broker.publish` sites in srv | 177 |

Each subscriber states its own guess at the payload, as a cast (`(event as MuxEvent)?.data?.values`) or as an inline parameter type (`(event: { data?: InstallChunk }) => …`). Nothing checks either side, so the RPC drift problem is open here:
- **Silent breaks:** a payload field renamed in srv breaks its subscriber at runtime, and no build or test fails.
- **Mixed payload code:** most payloads are built with `json!`, so srv has no type to generate from. Of the 33 events declared in `mps.rs`, 30 publish sites use `json!` and 55 serialize a value. The rest of the 81 are declared as string literals elsewhere.

## Design

Same shape as the RPC registry: srv records the type, a schema file is exported, and a generator writes the frontend binding.

1. **A payload trait in srv.** `trait EventPayload: Serialize + ts_rs::TS { const EVENT: &'static str; }` and `Broker::publish_payload<P: EventPayload>(&self, payload: &P, scopes: Vec<String>)`, which builds the `MuxEvent`.
   - **Registry:** each payload type is recorded in an event registry, alongside `RpcSchema`.
   - **Old path kept:** `publish(MuxEvent { .. })` stays, so migration can go one event at a time.
2. **Exported schema.** A test writes `frontend/types/rpc/event-schema.json`: event name to TS type name and its imports, like `rpc-schema.json`. `check-rpc-bindings.sh` already fails when that directory is stale.
3. **Generated map.** `scripts/gen-rpc-stubs.mjs`, or a sibling, writes `frontend/app/store/mps-events.generated.ts`, containing:
   - `type MuxEventPayloads = { controllerstatus: ControllerStatusEvent; … }`, and
   - typed overloads of `muxEventSubscribe`: for a migrated name, `handler(event)` gets `MuxEvent & { data?: MuxEventPayloads[K] }`.
   - **Unmigrated names** keep `any`, so nothing breaks partway through.
   - **Name constants:** `mps-events.ts`'s constants are checked against the schema's names.
4. **Signals with no payload.** An event that only says "something changed" is declared with payload `()`. It goes out with no `data` field, as before, and `onMuxEvent` calls its handler with `null` (`NO_PAYLOAD_MUX_EVENTS` in the generated file).
5. **Ratchets.** A gate counts `json!` payloads in `publish` calls and casts of `event.data` in subscribers. Both may only go down, like `check-untyped-rpc-handlers.mjs`.

## Order

One PR per domain, about 10 events each. The busiest subscribers go first:

1. **Infrastructure:** the trait, registry, schema export, generator and gate. Migrate `controllerstatus` (8 subscribers) and `sysinfo` (6) as the proof.
2. **Agent:** `agentturn`, `agentfailure`, `agentruntime`, `agentcontextusage`, `agent-message-accepted`, `agent-resume-retry`, `agent:shutdown`, `userattention`.
3. **Subagents and swarm:** `subagent:*`, `dispatch*`, `backgroundtask*`.
4. **Files and blocks:** `blockfile`, `blockstats`, `block:activity`, `files*`, `editor*`, `media*`, `waveobj:update`.
5. **Signals:** the fixed-name `*changed` events with no payload (agents, memories, skills, MCP, accounts, cron, drones, remotes, the work queue). The widget events move to step 7.
6. **Shell, install, attachments and connections:** `shell_*`, `install_*`, `attachment:*`, `connchange`.
7. **The rest**, including the widget events and the per-id names (`agent:memory:changed:{id}` and the like), and the gate at zero.

**Effort:** about 7 PRs, comparable to the stub work. Every step can be shipped on its own: an unmigrated event works exactly as today.

## Not in scope

- **Event names:** they keep their current wire strings, with no renames.
- **Delivery:** the transport and scoping don't change.
