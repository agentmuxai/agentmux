---
type: patch
---

feat(launch-modal): migrate bindings + subscribe to backend push events (Stage 2c.3)

Final piece of the launch-modal state-machine hardening.

- `selectedBundleBindings` createResource removed; the reducer
  emits `FetchBindings` on identity selection and the view's
  event sink runs the RPC + dispatches BindingsLoading/Loaded.
- Subscribes to backend's `identitybundlebindings:changed:<id>`
  event (already emitted on bundle_bind / unbind RPCs) so cross-
  tab modifications via the Identity pane update this modal
  without a manual refetch.
- `bundleHasMatchingBinding` memo delegates to the slice's
  `hasMatchingBinding(state, providerId)` selector.

Closes the Stage 2 spec — all of form / submit / resources /
bindings flow through the reducer slice now.
