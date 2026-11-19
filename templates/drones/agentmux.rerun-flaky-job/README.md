# Flag a flaky job's first failure

Every 10 minutes this looks at the repository's failed pull request runs on GitHub. A run of the workflow you name that failed on its first attempt is told once to the leader, with its link and a line saying to re-run its failed jobs. It starts nothing itself.

Change first: the repository (`repo`, as `owner/name`), the flaky workflow's name (`workflow`) and who is told (`leader`).

The Vault entry (`credential`) must hold a GitHub token that can read the repository's runs. The first call asks you once; the token is sent to api.github.com and nowhere else.

Why it does not re-run the job itself: it reads text nobody vouches for (a poll of GitHub), it uses a credential, and a re-run would write outside AgentMux. A template that does all three is refused until a run can wait for a person's approval, so for now the re-run is left to a person or an agent. When approvals exist, the version that re-runs the job comes back with `policy.approval` set to `required`.

A run is told once: the poll counts it as seen the first time. The first look only notes the runs that are already red.

Anyone who can open a pull request can make a run of the workflow fail, and so cause one line to the leader; the poll starts at most six runs an hour. The repository must be `owner/name` and the workflow's name holds no quote mark or backslash: both are refused when the drone is made.

Messaging the pull request's author needs a way to read an agent id out of the pull request's text, which this build does not have: the one line goes to the leader.

Test payload: turn it on, then make a pull request run of that workflow fail once, and wait up to 10 minutes.
