---
type: patch
---

Git facts of a container agent are read inside its container: its work facts, its overlap and PR notes and the git markers of its folder in the Files pane come from git run in the agent's container, for every folder the agent can write (its workspace, its memory folder, its writable volumes). No git is started on the host for such a folder; there are no facts while the container is not running.
