# SPEC: LAN key continuity, with no person in the loop

**Status:** active — approved by the operator (2026-10-29, relayed by AgentY); built in PR #5815 (0.60.7). Remains: the pin screen note and reason, retire notices, directory publishing (§6). Author: Lazo@Area54.
**Builds on:** `SPEC_LAN_AGENT_KEY_REPLACEMENT_2026_10_27.md` (option A, the install certifies the key), `SPEC_JEKT_LAN_TIER_SIGNING_2026_08_15.md` §2.2 (the name pin), `SPEC_WAN_JEKT_VERIFICATION_2026_09_24.md` (the install key, the directory and the presence list).

## 1. The problem

A LAN peer pins the first key it sees for an agent's name (and for its `(name, UID)`). A later, different key is refused and flagged until someone presses "Accept the new one" on every receiving machine. An agent's key changes for ordinary reasons (a new channel store, a recreated agent, a rebuilt install), and the cost is a click per machine per agent. One host's agents were refused on every other host for a day because of exactly that.

## 2. The rule

A different key for a pinned name is **accepted automatically, and the pin moved**, only when something this machine already trusts vouches for it. Otherwise the key stays refused, exactly as today, and leaders are told.

**The vouch.** The lookup answer that carries the new key also carries a certificate (`lan_cert`): a `WanKeyRecord` with channel `lan`, signed by the hosting install's instance key, over the agent's name, the key and the time. The receiver accepts when **all** hold:

1. the certificate's chain verifies (the instance id is the hash of the instance key, the fingerprint is the hash of the key, the instance key signed it), it names this agent, channel `lan`, and exactly the key the peer showed;
2. it is fresh (issued within ten minutes of now) and newer than the pin it replaces, so a copy of an old certificate cannot move a pin back to a key that was retired;
3. the issuing install is **trusted** here, by either:
   - **a pinned instance:** this machine already pinned that instance for some agent (the WAN instance pin), or
   - **the account's directory:** the signed-in account's install list shows that instance as a live install of the account, and it is not revoked (the same standard WAN verification applies to a sender).

The `(name, UID)` pins go with the old key: moving the name pin clears them, and the first message that verifies under the vouched name key pins the UID key again, beside the name pin as always. No separate UID certificate is needed, since a UID signature is only checked after the name signature verified.

## 3. What stops an impostor

- **On the LAN, without the account** (the case the pin exists for): the vouch needs a signature by an install key that is pinned here or listed in the account's directory. The attacker has neither private key, so they cannot make a certificate for a key of their own. Copying a real certificate gains nothing: it certifies the real key, whose private half they lack. Replaying an old one fails the freshness and newer-than-pin checks.
- **Path "pinned instance":** the attacker would need that install's private key, which never leaves its machine and which the agent never holds (so an agent whose key leaked cannot vouch for a replacement).
- **Path "account's directory":** the attacker would need to be signed in to the account and to have published their own install. That is the same boundary WAN verification already treats as trusted, so it adds no new exposure; it is also why every automatic move is audited and alerted (§4). When an approved-installs list exists, this path narrows to approved installs only.
- A moved pin never marks a message verified by itself: the message signature must still verify under the new key.

## 4. Audit and alert

Every automatic move writes one line to `logs/pin-changes.log` (who accepted what and why): the agent, old and new fingerprint, the issuing instance (id and host hint), which trust path vouched, and the certificate time. The same event is a warning in the srv log: a line for leaders, not a modal. A refusal without a vouch is kept in memory with its reason ("not vouched: <why>") beside the refused key, and the agent route returns that reason. (Showing the note and the reason on the Pinned senders screen needs new wire fields, so it is a follow-up.)

## 5. The agent route

`pins.list` and `pins.adopt` (kind `lan`) accept an agent's connection as well as the user's window. For an agent, `pins.adopt` accepts the refused key only when the vouch above holds *now* (srv checks the refused key's certificate itself; the caller's word counts for nothing), and says why when it does not. The user's window keeps its unconditional "Accept the new one". `pins.forget` stays with the user's window: forgetting a pin makes the next key shown for the name first-seen, which is exactly how an impostor would get in, and there is no proof to ask for. If an agent ever needs it, it gets the same vouch rule. The audit line records the caller.

## 6. Out of scope

Retire notices for keys the install retired (a refused key can be told apart from a retired one later); publishing certificates to the directory so a peer that was offline learns without a lookup; the install approval list. Hosts that do not send certificates (before this release) behave as today: refused until accepted by hand.

## 7. Tests

An impostor key with no certificate is refused and the pin stays; a certificate by an untrusted instance is refused; a vouched rotation moves the name pin and the UID pin; the audit line is written with the path and the fingerprints; a stale certificate, a certificate older than the pin, a certificate for another agent or another key, and a certificate whose chain is broken are each refused; the agent route accepts a vouched key and refuses an unvouched one; a host that sends no certificate is unchanged.

Agent: Lazo@Area54
