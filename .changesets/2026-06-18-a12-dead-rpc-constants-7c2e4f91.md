---
type: patch
---

refactor(A12): remove 65 dead COMMAND_* constants from rpc_types.rs

These constants were defined but never referenced by any backend handler
(`register_handler` call) anywhere in the codebase. They are legacy
Wave/WaveTerm RPC surface that was never ported to the agentmux backend:
file/remote/conn/WSL/VDOM/AI send message/VDOM/stream-test/activity/etc.

The A1 contract test (PR #1544) passes unchanged — the removed constants
were not registered handlers, so no baseline changes are needed.

Part of the A1–A15 architecture refactor board (A12 dead-code sweep).
Previous sweep removed StreamStalled watchdog in #1542.
