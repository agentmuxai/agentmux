# Triage a failed nightly

Every 15 minutes this looks at the repository's scheduled workflow runs on GitHub. For each failed run it has not seen, it reads the failed job's log and an agent names the cause, the suspect commit, the area and how sure it is. Unless the agent is unsure, a Kanban task is filed for the triager, and a red night that repeats adds a line to the same task. The leader gets one message.

Change first: the repository (`repo`, as `owner/name`), who is told (`leader`) and who gets the task (`triager`).

The Vault entry (`credential`) must hold a GitHub token that can read the repository's Actions runs. Give it read access only. The first call asks you once; the token is sent to api.github.com and nowhere else, and never reaches the agent.

The first look only notes the runs that are already red; it starts a drone run only for failures after that. The agent runs only on a failure, so waiting costs no tokens. The log is outside text: the agent is told to treat it as data, and its answer is four short fields, never prose.

Test payload: turn it on, then re-run a scheduled workflow that fails on purpose, and wait up to 15 minutes.
