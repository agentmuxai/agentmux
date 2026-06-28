---
type: patch
---

fix(app-api): preset.upsert accepts the documented request shape

The handler deserialized straight into the `Memory` struct, so the spec's
request shape failed: array `context_files`/`mcp_servers`/`skills` hit
"invalid type: sequence, expected a string", and omitting `id` hit
"missing field `id`". The request body is now normalized first — array
fields are re-encoded to their JSON-string form and a missing/null `id`
defaults to empty (mint-on-create) — so the API behaves exactly as the
spec documents. Part of #1836.
