# TRACKING — WAN jekt verification (same-account, W3-S)

**Date:** 2026-09-25
**Type:** Tracking doc for `SPEC_WAN_JEKT_VERIFICATION_2026_09_24.md` — where
each step stands, what is live, what still needs a person, and what is
deliberately not built. Not a design spec.
**Status:** living — updated with every W3-S PR and after the C1 deploy.
**Tracks:** issue #2586 (WAN half). Design of record for the cross-account
phases W0–W2: `SPEC_JEKT_WAN_TIER_SIGNING_2026_09_17.md`.

---

## 1. Step status

| Step | What | PR | State |
|---|---|---|---|
| — | Spec (three adversarial reviews) | #3649 | merged |
| — | Pure primitives: instance id, key fingerprint, instance-signed certificate, revocation, envelope + freshness checks, cross-language vectors (spec §2.7) | #3727 | merged |
| C1 | Cloud: carry the eight `wan_*` fields, store the sender account (never returned), `sender_same_account`, idempotent `(account, wan_msg_id)`, key directory + revocation routes with chain checks, `muxbus-agent-wan-keys-<env>` table | agentmux-cloud#91 | merged, **not deployed** |
| D1a | Channel-wide `wan.db`, instance key, agent WAN keys moved into it (survive upgrades), instance id as `AGENTMUX_HOST_LABEL`, agent-delete purge reaches `wan.db` | #3734 | merged |
| D1b | Certify + publish each agent key (`muxbus/wan_publish.rs`), relay carry gate (`relay::wan_carry_gate`) | #3771 | merged |
| D2 | Verifier (`muxbus/wan_verify.rs`), peer cache, known instances, replay table, `TRUST=wan-verified` marker, tier rules, audit, `wan` grants off | #3775 | merged |
| — | Agent jekt policy (`~/.agentmux/agents/CLAUDE.md`) gains `TRUST=wan-verified` | — | **needs the operator** (§3) |
| — | Host-gated instance approval window | — | **held** for GHSA-6726-q276-g6f6 (§4) |
| — | Instance retirement from the desktop | — | not started (§4) |
| — | End-to-end run across two machines (spec §5) | — | **blocked on the C1 deploy** |

## 2. What is live on `main` today

- Every channel mints one instance keypair on first boot; its 26-char id is
  what agents now sign as. Respawned agents get their WAN key from `wan.db`
  (an existing `objects.db` key is imported once).
- The publisher PUTs each agent's certified key to the cloud directory. Until
  C1 is deployed the cloud answers 404, so it retries **hourly** and nothing
  is published.
- The relay carries a signature only when the sender is host-verified, signed
  as this instance and channel, verifies under its `wan.db` key, and that key
  is confirmed published. Until C1 is deployed, nothing is published, so
  nothing is carried: **every WAN jekt still arrives `TRUST=network-claimed`,
  exactly as before.**
- The receiver verifies whatever arrives carried. With nothing carried, it
  changes nothing yet.

## 3. Needs a person

1. **Deploy C1** — `agentmux-cloud` `deploy.yml` (`workflow_dispatch`, manual
   approval). Before relying on it, check `/api/health` reports server
   `1.9.0` or later, and that `cdk diff` shows exactly one new table, one env
   var and one grant.
2. **Apply the jekt-policy text** proposed in the #3775 description to
   `~/.agentmux/agents/CLAUDE.md`. That file is outside the repo and tells
   agents not to trust unconfirmed edits to it, so the implementing agent did
   not edit it.
3. **Respawn agents** after the deploy, so each picks up its `wan.db` key and
   instance label. Agents spawned before D1a sign a hostname and are sent
   unsigned by the carry gate until respawned.
4. **Run the §5 end-to-end check** with two machines on one account: jekts
   both ways show `TRUST=wan-verified INSTANCE_STATUS=new` (the other
   install) — `approved` only for a message verified by its own install;
   after an upgrade it is still the same instance; from a second account,
   `TRUST=network-claimed`.

## 4. Deliberately not built

| Item | Why | Unblocks when |
|---|---|---|
| Approval window (another install → `approved`) | Must be reachable only from the CEF host, not by agents holding `X-AuthKey`; the spec's §2.6 amendment holds it until GHSA-6726-q276-g6f6 is fixed. Until then no other install relaxes a stop. | the advisory is fixed |
| Instance retirement | Needs the same host-gated surface as approval. The cloud already accepts revocations (C1) and the verifier honours them. | with the approval window |
| Honouring `wan` trusted-peer grants | Grants are keyed by bare name; on WAN one name can be several instances. Needs an instance-keyed grant (table rebuild) and a way to create grants. | spec 09-17 §3.5.1 |
| Cross-account verification (W0–W2) | Out of W3-S's scope by construction. | spec 09-17 |

## 5. Found during implementation

- **Concurrent first open of `wan.db` failed with SQLITE_BUSY** (8 concurrent
  opens, 3/3 runs): SQLite skips the busy handler where waiting could
  deadlock. The one-time setup is now retried within the 5 s bound (#3734).
- **Replay-row pruning used the wall clock** while freshness used the
  verifier's `now`, so a row could be pruned while its message could still
  verify. One clock throughout, with a boundary test (#3775).
- **Record match was case-sensitive for instance and channel**, against
  spec §2.3 — Codex P2 on #3727, fixed there.

## 6. Open questions carried from the spec

- §6.4 — stop injecting `MUXBUS_TOKEN` into agent environments (would make
  instance minting srv-only).
- §6.5 — keep the instance key out of agents' reach (OS keychain / separate
  user); today a copied `wan.db` lets its holder speak as that instance until
  it is retired.
- §6.6 — whether per-version `objects.db` also re-mints host and LAN keys on
  every upgrade.
- Purge scope (#3734): the "name still in use" check reads this version's
  `db_agents`, not every version of the channel. A miss degrades to unsigned
  until respawn, never to a false alarm.
