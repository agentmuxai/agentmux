---
type: patch
---

A new Kanban task can start as a spec: Spec first (on by default) files a spec and a build that starts after it; when the spec is done the build's agent is told it can start, the build's card links the spec's document, and cancelling a spec cancels or holds its build.
