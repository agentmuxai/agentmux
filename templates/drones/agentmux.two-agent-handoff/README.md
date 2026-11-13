# Two-agent hand-off

The first agent does a piece of work. The second agent gets the first one's answer quoted into its task and does the next piece. The second answer is sent to an agent of yours as a message.

Change first: the two pieces of work (`first`, `second`) and who gets the result (`owner`). It runs when you start it; give the job a schedule in the Jobs tab if you want one. Each run is capped at 3 dollars across both agents.

Test payload: run it once by hand with two small tasks, such as "name three colours" then "put them in alphabetical order".
