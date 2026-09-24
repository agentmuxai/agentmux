# SPEC: Generic integrations — external services talk to agents through one authenticated interface, nothing hard-coded

**Date:** 2026-09-24
**Status:** draft. This is a skeleton, and research is in progress. It will
hold the full research and the design, and it gets an adversarial review
before anything is built.
**Trigger:** the repo owner, 2026-09-24: *"we need generic github
integrations. nothing hard coded … ideally the agentmux interface is
generic and reagent has the consumer/API rights with auth, so any service
could come in later too. keep in mind 'connect to github' work that is in
progress."*
**Scope:** both repos.
- `agentmux`: desktop srv, MCP and frontend.
- `agentmux-cloud`: the muxbus relay, and the GitHub consumer that
  currently lives in it.

The external services come in as clients: ReAgent (`a5af/reagent`) first,
any other later.

## 0. Why

Today a ReAgent- and GitHub-specific notification path is compiled into
both products:
- a GitHub consumer Lambda inside agentmux-cloud that signs with a
  `reagent-v1` key;
- `reagent_*` fields on the relay's inject rows;
- ReAgent public keys pinned in the desktop binary;
- a ReAgent-specific `SIG=` marker and tier rule.

Adding any other service would mean more hard-coded paths, and each one is
a place trust can go wrong: the exposed dev key fixed on 2026-09-24 was
exactly that.

## 1. Research (in progress)

1.1 The current GitHub → agent notification path (cloud consumer, relay
fields, desktop verification) — *to be filled*
1.2 ReAgent (`a5af/reagent`): architecture, how it reaches agents today —
*to be filled*
1.3 In-progress "connect to GitHub" work — *to be filled*
1.4 Everything ReAgent- or GitHub-specific in both repos — an inventory
for removal — *to be filled*

## 2. Design (to follow)

The target shape, to be specified against the research:
- **agentmux exposes one generic integration interface.** It names no
  service. An integration is a registered client with its own credential,
  its own scopes (which agents or accounts it may message, and how), and its
  own identity shown in the marker.
- **Services own their logic.** ReAgent turns GitHub events into
  notifications itself and calls the interface with its credential. The
  GitHub consumer leaves agentmux-cloud, or becomes a generic service
  outside it.
- **Trust comes from the credential, not pinned keys.** Nothing about any
  service is compiled into the desktop.
- **"Connect to GitHub"** — the user's own GitHub link — fits this model as
  one integration among several, not as a special case.

## 3. Migration, removal, rollout (to follow)

## 4. Open questions (to follow)
