# SPEC: telling LAN peers that an agent's key was replaced

**Status:** draft for AgentY (follow-up to narko Kanban #5592). Spec only, nothing built. Author: Lazo@Area54, 2026-10-27.
**Builds on:** `SPEC_JEKT_LAN_TIER_SIGNING_2026_08_15.md` (§2.2 the name pin), `SPEC_JEKT_HOST_KEY_TTL_ROTATION_2026_09_14.md` (§4, why LAN keys were never rotated), `SPEC_LAN_JEKT_SIGNED_BY_SRV_2026_10_25.md`, `SPEC_WAN_JEKT_VERIFICATION_2026_09_24.md` ("Replaced keys", the WAN notice this spec reuses).

## 1. What happens today

A LAN peer learns an agent's key from the install that hosts it: `GET /agentmux/reactive/agent?id=<name>` answers with `lan_public_key`, and with `uid` and `uid_public_key` for the v2 signature (`LanDiscoveryController::find_agent_lan_pubkey`; answers are cached for 60 s). The receiver then pins what it saw:

- the **name pin**, `db_lan_peer_pubkey_pins`: the first key seen for a name, forever (`lan_peer_pubkey_pin_get_or_set`);
- the **UID pin**, `db_lan_peer_uid_pins`: the first key seen for a `(name, UID)`, set only beside a matching name pin.

A later, different key for the name is `lan_verified = Some(false)` (forced sensitive tier), the key is remembered in memory as a refusal (`lan_refusals`), and nothing else happens. The only way out is the operator's pin screen on **each** receiving machine (`pins.list` / `pins.forget` / `pins.adopt`, operator-only, logged to `pin-changes.log`).

So when an install replaces an agent's LAN key (the container-agent key replacement in #5595 is the first case; an agent delete and re-create is another), every peer that ever pinned the old key flags that agent's LAN messages until its owner adopts the new key by hand. There is no way for the install to say "this was me".

What a LAN peer can and cannot rely on:

- **No authenticated install identity.** The `instance_id` in the LAN advertisement is the version string (`v0.59.7`), not a key. Discovery is unauthenticated (the reason for the pin).
- Every install **does** have an instance keypair in `wan.db` (id = hash of the key, `wan_instance_id`), used to certify WAN agent keys. LAN does not use it.
- The WAN side now has a signed notice for a retired key (`WanKeyRevocation`: instance, agent, channel, fingerprint, signed by the instance key).

## 2. What has to be true

1. A peer must be able to tell "the install that hosts this agent replaced its key" from "someone else shows a different key for this name". The first is routine; the second is the forgery the pin exists to catch.
2. It must work when the old key should no longer be trusted: a handover signed by the old key proves nothing then.
3. A peer that was offline, or that pinned long ago, must still get the answer when it next sees the agent. LAN has no server to hold state, so the answer travels with the lookup.
4. Nothing may loosen the pin for peers that learn nothing: the default stays "different key = forced sensitive, operator decides".

## 3. Options

### A. The install certifies the agent's LAN key (reuse the instance key)

The lookup answer also carries a certificate: `instance_pubkey` and a signature by it over (instance id, agent, channel `lan`, key fingerprint), exactly the `WanKeyRecord` shape with channel `lan`, for the name key and for the UID key. A receiver pins `(name → key, instance id)` together. Replacement is then recognisable: the new key arrives with a certificate by the same instance that the pin is bound to, and a retired-key notice (`WanKeyRevocation` with channel `lan`, no new signing code) for the old fingerprint can accompany it.

- Works when the old key is exposed or gone: the authority is the install's key, which the agent never holds.
- Automatic for peers that bound the instance; no per-machine action.
- **Cost.** Lookup answer grows (`lan_cert`, `uid_cert`, optional `retired`). Pin schema gains an instance column, plus a binding step for existing pins (§5). Receivers verify two more signatures per first lookup. The instance key becomes a LAN trust anchor, so losing `wan.db` (a new instance id) looks to LAN peers like a different install and falls back to the pin screen. About 2 PRs.
- **Limit.** Anchors on the first sight of the install, like the pin itself. It does not help a peer that first meets the agent through an impostor, but neither does anything else on an unauthenticated LAN.

### B. A handover signed by the old agent key

The old key signs "K_old → K_new"; the answer carries the chain, and a receiver pinned to K_old accepts K_new.

- Needs no new key and no wire change beyond one field.
- **Does not work when the old key should no longer be trusted:** whoever holds it can sign a handover to their own key, which is exactly the case that prompted this. It also needs the old private key to still exist when the new one is minted; the replacement paths delete it first.
- Right for routine rotation only (a LAN key TTL), which does not exist and was declined in the TTL spec for this very reason.
- **Cost.** Keep the old key until the handover is made, chain storage and verification. About 1 PR, and it only covers the case nobody has.

### C. The pin screen (what exists)

`pins.adopt` pins the key last refused for a name.

- Zero new protocol; works for every case; the operator is the authority, which is the safest.
- **Cost is per machine and per rotated agent**: with N peers and M replaced agents, N×M manual adoptions, discovered only when messages arrive flagged. In the meantime the agent's LAN messages are forced to the sensitive tier everywhere.
- Small improvements are cheap and independent of A or B: show on the refusal row that the new key has a valid certificate (once A exists), batch "adopt all refused for host X".

### D. Carry the notice through the account's key directory

For peers of one account, publish LAN fingerprints and retirements to the same directory the WAN notices use, and let receivers read its feed.

- Reuses the directory and the feed read built for WAN; a peer that was offline still learns.
- Only for signed-in, same-account peers, and a LAN-only machine has no feed. It needs A's certificate anyway to know who may retire what.
- Best as an add-on to A (§4), not an alternative.

### E. Do nothing

Keep C as the only path. Acceptable while replacements are rare (a one-time container-key replacement), not once any automatic replacement exists.

## 4. Comparison

| | Old key exposed | No per-machine action | Offline peer catches up | New wire fields | Pin schema | Rough size |
|---|---|---|---|---|---|---|
| A. instance certifies | works | yes, after binding | yes, via the next lookup | certs, `retired` | + instance id | 2 PRs |
| B. signed handover | **no** | yes | yes | chain | + chain | 1 PR |
| C. pin screen | works | **no** | n/a | none | none | exists |
| D. directory feed | works (with A) | yes, same account | only while signed in | none | none | 1 PR on top of A |

## 5. Recommendation

**A, with C kept as the fallback, B not built, D later if wanted.**

Why: replacement happens because a key should no longer be trusted, and only an authority the agent never held can say so. A is the only automatic option that has that, and it costs one new column and two certificates because the install key and the notice format already exist.

Plan, each step shippable alone:

1. **Certify in the answer.** The hosting install adds `lan_cert` (and `uid_cert`) to the `agent?id=` answer. Old receivers ignore unknown fields. Nothing changes for anyone yet.
2. **Bind on first verified message.** A receiver stores the certificate's instance id with the pin only when the message in hand also verified under the pinned key. A certificate alone is not enough: anyone can wrap a public key in a certificate of their own instance, so binding an old pin on the strength of a lookup answer would let an impostor become the pin's "owner". Binding on a message signed by the pinned key makes the first binding as strong as the pin was. Pins made before step 2 stay unbound (pin screen) until their agent next sends a verified message.
3. **Move the pin on a certified replacement.** When the observed key differs from a pin that is bound to instance I, move the pin (name and UID) automatically only if all hold: the new key's certificate verifies and is by I, the agent and channel `lan` match, and the certificate is newer than the one the pin was bound with. Log it to `pin-changes.log` like an adopt, with reason `replaced-by-install`. Anything else stays a mismatch (forced sensitive, pin screen).
4. **Retire notices.** The same answer carries `retired: [WanKeyRevocation…]` (channel `lan`) for fingerprints I retired, so a peer that meets a stale impostor key can tell it is retired rather than merely different, and a peer whose pin is a retired key is told so on the pin screen.
5. *(Optional)* **D**: publish retirements to the directory so signed-in peers learn them without a lookup.

The agents already replaced before step 3 exists have unbound pins everywhere; they take the pin screen once. That is the price of doing this after the fact, and it is why steps 1 and 2 should ship soon even though 3 is where the benefit shows.

## 6. Security notes

- Binding is the weak point (step 2); the rule above keeps it no weaker than the existing trust-on-first-use pin.
- An attacker on the LAN who copies a real certificate gains nothing: it certifies a key they do not hold the private half of.
- A peer that accepts a replacement still requires the message signature to verify under the new key; the pin moving never marks a message verified by itself.
- A compromised install (its instance key) can certify anything; that is the same trust a WAN peer already places in it.
- The forced-sensitive tier stays for every message between "key replaced" and "pin moved", so the gap fails toward more care, not less.

## 7. Open questions

1. ~~Should a pin move automatically at all, or only offer "adopt"?~~ **Decided (AgentY, 2026-10-27):** automatic, with a visible notice ("pin for X moved to its new key, certified by the same install"), whenever the replacement is certified by the instance already bound to that pin; "adopt" is offered only when that is not proven. The install's key is a better authority than a click on an alert the operator cannot tell from an attack. The notice goes in the same place the pin screen lists changes (and `pin-changes.log`), not in the agent's conversation.
2. The UID pin: does a certified name replacement also move the `(name, UID)` pin? Recommendation: yes, together, only if the UID is unchanged.
3. Which clock orders certificates (`issued_at` of the key, as WAN does)? Recommendation: the same.
4. Do we want the LAN lookup answer signed as a whole (so a peer can tell an answer from a relay)? Out of scope here; it would also close the first-contact race.

## 8. Tests (when built)

A certificate by another instance does not move a pin; an unbound pin never moves automatically; binding needs a verified message; a replayed old certificate cannot move a pin back; the UID pin moves with the name pin or not at all; old receivers and old hosts interoperate (fields ignored); the pin log records every automatic move.

Agent: Lazo@Area54
