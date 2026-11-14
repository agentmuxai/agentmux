---
type: patch
---

Core tests: route_credits and worker_notes tests run commands through the core fixture's reducer state instead of server::AppState, and the worker-of content type comes from backend::agent_seat (prep for the srv-core split)
