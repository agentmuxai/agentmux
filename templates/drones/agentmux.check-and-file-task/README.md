# Check and file a task

An agent checks something every six hours and answers `OK` or describes the problem. Only a problem files a Kanban task; a problem that is still there on the next check adds a line to the same task.

Change first: what to check (`check`). Leave `target` empty for the backlog, or give an agent id.

The condition compares the agent's whole reply with `OK`, so keep that instruction in the check: an answer of "OK." counts as a problem.

Test payload: run it once by hand from the Jobs tab. Make the check fail on purpose (ask about something that is not true) to see the task appear.
