# Re-run a flaky job once

Every 10 minutes this looks at the repository's failed pull request runs on GitHub. A run of the workflow you name that failed on its first attempt is started again, only its failed jobs, and the leader gets one line.

Change first: the repository (`repo`, as `owner/name`), the flaky workflow's name (`workflow`) and who is told (`leader`).

The Vault entry (`credential`) must hold a GitHub token that can read the repository's runs and re-run its workflows. The first call asks you once; the token is sent to api.github.com and nowhere else.

The only thing it writes outside AgentMux is the re-run, and the run's number is a number the poll read, so nothing a person wrote can steer it. A run is never reported twice, so a run is never re-run twice from this computer. Two computers on one account that both turn it on may both re-run it.

Anyone who can open a pull request can make a run of the workflow fail, and so cause one re-run of its failed jobs; each run is re-run once at most, and the poll starts at most six runs an hour. A re-run uses the privileges of the person who started the original run, not of this token (GitHub's documentation on re-running workflows), so it gives a pull request from a fork no more rights than its first attempt had; it does use CI minutes. The repository must be `owner/name` and the workflow's name holds no quote mark or backslash: both are refused when the drone is made.

Messaging the pull request's author needs a way to read an agent id out of the pull request's text, which this build does not have: the one line goes to the leader.

The first look only notes the runs that are already red.

Test payload: turn it on, then make a pull request run of that workflow fail once, and wait up to 10 minutes.
