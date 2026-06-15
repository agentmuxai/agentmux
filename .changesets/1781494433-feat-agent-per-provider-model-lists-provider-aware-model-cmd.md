---
type: minor
---

feat(agent): per-provider model lists + provider-aware `/model` — the `/model` picker now shows the active provider's models (Claude opus/sonnet/haiku, Codex gpt-5.5/gpt-5.4/gpt-5.1-codex-max/gpt-5.3-codex) and Codex honors the picked model; runtime `model` is now provider-scoped (P2 + model-side of P3 from SPEC_PROVIDER_MODELS_EFFORT_GENERALIZATION)
