# SPEC: Generate the Rust ↔ TypeScript RPC bindings from srv

**Status:** proposed — the design for Phase 2 (steps 7–9) of `docs/reports/REPORT_DRY_AND_MODULARITY_AUDIT_2026_09_06.md`. No generator exists yet; this is the plan and the reasons for its shape.

## 1. Problem

The frontend talks to `agentmux-srv` through 284 hand-written command stubs (`frontend/app/store/rpc-api/*.ts`), 38 service methods (`frontend/app/store/services.ts`) and a 2,819-line hand-maintained `frontend/types/gotypes.d.ts`, each headed "keep in sync with…". On srv's side there are about 190 `#[derive(Serialize, Deserialize)]` structs under `backend/rpc_types/` and 237 `engine.register_handler(COMMAND_X, …)` registrations. Nothing checks that the two sides agree. The audit counted 334 sync comments and found one live drift by hand (the default layout tree, fixed in PR #2988, whose frontend copy claimed to be in sync and could not even express the backend's sizing).

## 2. Why "derive the TypeScript types" is not enough on its own

The obvious tool — `ts-rs`, `#[derive(TS)]` on every `rpc_types` struct — produces the *types* but not the *bindings*. Which command takes which request struct and returns which response is not data anywhere in srv: each handler closure picks its request type inline (`serde_json::from_value::<CommandXData>(data)`) and builds its response ad hoc (`serde_json::to_value(&filtered)`, `json!({ "ok": true })`). The frontend's stubs reflect that:

| stub return type | count |
|---|---|
| `Promise<void>` | 70 |
| `Promise<{ deleted / bound / unbound / ok: boolean }>` | 25 |
| a named `gotypes` type (`AgentDefinition`, `Skill`, `McpServer`, …) | ~60 |
| an inline object literal | the rest |

So the generator needs a **command registry** first: command name → request type, response type. That registry does not exist. Creating it is the real work, and it is also what makes the handlers themselves type-checked.

## 3. Design

### 3.1 Typed registration in srv

Add alongside `RpcEngine::register_handler`:

```rust
pub fn register_typed<Req, Resp, F, Fut>(&self, command: &'static str, f: F)
where
    Req: DeserializeOwned + TS,
    Resp: Serialize + TS,
    F: Fn(Req, HandlerCtx) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<Resp, String>> + Send + 'static,
```

It wraps `f` in today's `CommandHandler` (deserialise `Req`, run, serialise `Resp`) **and** records `(command, Req::name(), Resp::name())` in an `RpcSchema` registry held by the engine. The untyped `register_handler` stays; handlers migrate one at a time, and `()` or `serde_json::Value` are legal `Req` / `Resp` for the few whose shape is genuinely dynamic.

### 3.2 Emission

A `--dump-rpc-schema <dir>` mode of `agentmux-srv` builds the engine with every handler registered and writes:

- `frontend/types/rpc.gen.d.ts` — every `#[derive(TS)]` type reachable from the registry, as module exports rather than `declare global`, so nothing collides with `gotypes.d.ts` while both exist;
- `frontend/app/store/rpc-api/gen/<domain>.ts` — one stub per registered command with today's call syntax (`RpcApi.XCommand(client, data, opts)`), typed with the generated types;
- `frontend/app/store/rpc-constants.gen.ts` — shared constants (step 8), so `persistent.rs`'s "no shared constant crosses the Rust/TypeScript boundary" stops being true.

`ts-rs`'s `serde-compat` feature honours the `rename_all`, `default` and `skip_serializing_if` attributes the structs already carry. The one known mismatch: a `#[serde(default)]` field is always present on the Rust *output* side but optional on the *input* side, so request structs get `#[ts(optional)]` wherever the frontend omits the field today. The generator's first run reports which of the 105 `#[serde(default)]` fields the existing stubs omit, so the annotation pass is a checklist, not a guess.

### 3.3 The gate

`scripts/check-rpc-bindings.sh`: run the dump into a temporary directory and `diff -r` it against the committed generated files; any difference fails CI. Same "is the generated artefact current" pattern as the specs-index gate. From then on, a Rust type change that is not reflected in the frontend is a build failure instead of a comment.

### 3.4 Migration order

1. Land `register_typed`, the `RpcSchema` registry, the dump mode and the gate with **zero** migrated handlers. The gate passes trivially; no behaviour changes. One PR.
2. Migrate handlers domain by domain, deleting the hand-written stub as each lands and switching callers to `gen/`. `agent.ts` first (51 stubs, the richest types); `file.ts` last (49 stubs, streaming edge cases).
3. When `rpc-api/*.ts` is empty, delete it and the `gotypes.d.ts` declarations that only the stubs referenced.
4. Step 9: the default layout tree, the provider catalog's `pinned_version`, and the two formatter mirrors (`format_global_brain_block`, `dim_agent_color`) become backend-owned data — served over an RPC or emitted as generated constants — instead of a Rust copy and a TypeScript copy each promising to match.

## 4. Non-goals

- Replacing the wire format or `RpcClient.rpcCall` — generated stubs keep calling it.
- Generating `services.ts` — it fronts a different, object-service protocol; at 38 methods it can follow the same registry once the command stubs are done.
- Touching `agentmux-cef`'s copy of the provider catalog — that is step 9, after the frontend's copy is gone.

## 5. Risks and how each migration PR answers them

- `ts-rs` adds a proc-macro to srv's build; the cost is small but should be measured once in the first PR.
- Migrating a handler can surface a request field the frontend never sent, or a response field it never read. Each migration PR diffs the generated stub against the hand-written one it replaces and explains every difference in the PR body.
- The dump mode must register every handler without a live `AppState`. If registration turns out to need one, the registry moves to a `const` table next to `commands.rs` — less elegant, same gate.
