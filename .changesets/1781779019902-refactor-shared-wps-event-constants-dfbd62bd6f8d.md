---
type: patch
---
Refactor: shared WPS event-name constants in frontend. Add wps-events.ts mirroring wps.rs constants; replace 25+ bare string literals across 12 files with typed WpsEvent.X references (A14)