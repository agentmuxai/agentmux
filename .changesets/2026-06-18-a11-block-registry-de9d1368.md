---
type: patch
---

refactor(A11): extract BlockRegistry + ModalLayer dispatch to dedicated modules

Moves the hardcoded view-type → ViewModel class map out of `block.tsx`
into `block-registry.ts`. Moves the concrete modal panel imports and
`renderRequest`/`requestLabel` switch out of `ModalLayer.tsx` into
`modal-dispatch.tsx`. Adding a new block view or modal kind now requires
editing only the dedicated registry/dispatch file — not the framework files.
