# Verify a release, then merge the held pull request

Every 10 minutes this looks at the repository's five newest releases on GitHub. Two and a half hours after the poll first sees a release (the trigger's delay), it fetches the release again and checks that it is not a draft or a pre-release and that its update manifest is among the assets. The leader gets one line whatever it finds. When the manifest is there, a card appears in the Drone pane with the proposed merge and the release's notes (marked as outside text); only your yes merges the pull request. Silence rejects after a day, and a rejection ends the run.

Change first: the repository (`repo`), the held pull request (`held_pr`, its number) and who is told (`leader`). `manifest_asset` is the file name the release must carry (`latest.json` unless you say otherwise).

The Vault entry (`credential`) must hold a GitHub token that can read the releases and merge the pull request. The first call asks you once; the token is sent to api.github.com and nowhere else.

What it does not do, against the full acceptance case in the Drone spec (§9 c):

- It does not run the repository's release-check script. The check is the one it can make from the release itself: not a draft, not a pre-release, the manifest asset present.
- It merges one pull request, the one you name, not a list.

Why this is allowed to merge: it reads text nobody vouches for (a poll of GitHub), it uses a credential and it writes outside AgentMux, which a template may do only with a person in between. The merge is behind an Approval block on its only path, and the turn-on check refuses the drone if that stops being true. The pull request number and the repository are the template's own values, not text from the release.

The release's tag, notes and link are outside text: they go into the leader's message as data and into the card marked as outside text, never into the merge call.

Test payload: turn it on with `held_pr` set to a pull request in a scratch repository, publish a release there with the manifest asset, and wait two and a half hours.
