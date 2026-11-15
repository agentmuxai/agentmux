# After an upgrade: health check

Five minutes after AgentMux starts on a newer version, this drone looks at this computer: every agent that should be running is idle, working or waiting for you, cloud messages (MuxBus) are connected, and every other AgentMux the LAN showed answers. If all is well, it sends the leader one line. If not, it files a Kanban task for the steward that names what is not up, and shows a notification.

Change first: who to tell (`leader`) and who gets the task (`steward`), both agent ids.

It starts only when this start made an upgrade, not on a plain restart. The five minutes are the job's delay: raise it on a computer whose agents take longer to come back. Each computer runs its own copy, so install it on each one you want checked.

Test payload: run it once by hand from the Jobs tab. A run by hand has no upgrade, so the version words in its message stay unfilled; the checks themselves are real.
