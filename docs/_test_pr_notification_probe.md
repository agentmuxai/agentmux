# Test PR — notification routing probe

This is a throwaway file created solely to test that GitHub PR review
notifications route back to the correct agent (`Lark`) when the PR was
pushed via the shared `GenericAgentX-asaf` fallback account (no dedicated
PAT registered on this host) and the `<!-- agentmux:agent_id=lark -->` tag
is present in the PR body, per `CLAUDE.md`'s MuxBus Identity section.

This PR will be closed without merging once the notification is confirmed.
Safe to delete.
