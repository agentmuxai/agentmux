---
type: patch
---

A container agent's memory folder is named from its workspace path alone, never from files inside the workspace. A workspace that is not a repository, or is a repository of its own, keeps the folder it had; a workspace below a repository's top, or a linked worktree, gets the folder of its own path in place of the repository's shared one.
