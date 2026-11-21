# A merged pull request whose task is still claimed

Every 10 minutes this looks at the repository's recently closed pull requests on GitHub. Thirty minutes after one was merged, if a Kanban task carrying that pull request's link is still claimed, the steward gets one line with the task, its title and who holds it.

Change first: the repository (`repo`, as `owner/name`) and who is told (`steward`).

The Vault entry (`credential`) must hold a GitHub token that can read the repository's pull requests. The first call asks you once; the token is sent to api.github.com and nowhere else.

It writes nothing outside AgentMux. The line goes to the steward you name, not to the task's holder: a message's recipient is written in the graph and never taken from a value, so the holder is named in the text.

The first look only notes the pull requests that are already closed. A pull request closed without a merge is skipped.

Test payload: turn it on, merge a pull request whose task you leave claimed, and wait about 40 minutes.
