# SPEC: Rename WSH Files to RPC

**Date:** 2026-04-17
**Status:** Draft

---

## Problem

Six frontend files use the prefix "wsh" — a legacy naming artifact from
WaveTerm (the upstream project AgentMux forked from). "WSH" stood for
"Wave Shell" or "Wave Shell Handler" — neither term exists in AgentMux.

New agents and contributors see `wshclientapi.ts`, `WshClient`, `TabRpcClient`
and have no idea what "wsh" means. The files are actually the **WebSocket RPC
client layer** — they send typed JSON-RPC commands to the sidecar over a
WebSocket connection.

---

## Rename Plan

| Old File | New File | Purpose |
|----------|----------|---------|
| `frontend/app/store/wshclient.ts` | `frontend/app/store/rpc-client.ts` | Base RPC client class + response helper |
| `frontend/app/store/wshclientapi.ts` | `frontend/app/store/rpc-api.ts` | Typed RPC command wrappers (generated-style) |
| `frontend/app/store/wshrouter.ts` | `frontend/app/store/rpc-router.ts` | Routes RPC messages between browser tabs |
| `frontend/app/store/wshrpcutil-base.ts` | `frontend/app/store/rpc-util-base.ts` | Low-level RPC send/receive primitives |
| `frontend/app/store/wshrpcutil.ts` | `frontend/app/store/rpc-util.ts` | High-level RPC utilities (TabRpcClient init) |
| `frontend/app/view/term/term-wsh.tsx` | `frontend/app/view/term/term-rpc.tsx` | Terminal-specific RPC handlers |

## Symbol Renames

| Old Symbol | New Symbol | Used In |
|------------|-----------|---------|
| `WshClient` | `RpcClient` | Class exported from rpc-client.ts |
| `WshRouter` | `RpcRouter` | Class exported from rpc-router.ts |
| `TabRpcClient` | `TabRpcClient` | **NO CHANGE** — already correctly named |
| `RpcApi` | `RpcApi` | **NO CHANGE** — already correctly named |
| `wshRpcCall` | `rpcCall` | Method on RpcClient |
| `sendRpcCommand` | `sendRpcCommand` | **NO CHANGE** — already correct |
| `sendRpcResponse` | `sendRpcResponse` | **NO CHANGE** — already correct |

## Import Updates

| File | Import Count | Notes |
|------|-------------|-------|
| `wshclient` | 43 | Mostly `WshClient` type imports |
| `wshclientapi` | 39 | `RpcApi` imports (symbol stays, path changes) |
| `wshrpcutil` | 40 | `TabRpcClient` imports (symbol stays, path changes) |
| `wshrouter` | 4 | Internal router imports |
| `wshrpcutil-base` | 3 | Low-level imports |
| `term-wsh` | 1 | Single import in term.tsx |

**Total: ~130 import path updates** across the frontend codebase.

## Approach

1. Rename files (git mv)
2. Find-and-replace import paths across all `.ts` and `.tsx` files
3. Rename `WshClient` → `RpcClient` class + all references
4. Rename `WshRouter` → `RpcRouter` class + all references
5. Rename `wshRpcCall` → `rpcCall` method + all call sites
6. Verify with `tsc --noEmit`
7. Run existing tests

## Risk

Low — pure mechanical rename. No logic changes. `tsc --noEmit` catches
any missed references at compile time.

## Non-Goals

- Renaming the WebSocket connection itself (it's correctly named)
- Changing the RPC protocol or message format
- Renaming backend RPC handler names in Rust
