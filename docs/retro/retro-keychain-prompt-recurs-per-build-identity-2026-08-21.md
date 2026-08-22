# Retro: the "fixed" Keychain prompt came back on the very next local build

**Date:** 2026-08-21
**Area:** `scripts/package-macos.sh` (bundle identity), `agentmux-srv/src/identity/secret_store.rs`
**Context:** continues the same investigation as
[retro-macos-muxbus-keychain-prompt-storm-2026-08-19.md](retro-macos-muxbus-keychain-prompt-storm-2026-08-19.md)
and
[retro-macos-keychain-saga-lessons-learned-2026-08-20.md](retro-macos-keychain-saga-lessons-learned-2026-08-20.md).
Read those for the storm fix and its self-heal gap; this one explains a
separate recurrence reported immediately after, on a build that contains
all three prior fixes.

---

## 1. Symptom (as reported)

Local `main` was pulled fresh (HEAD `53e28b5b7`, well past `chore: release
v0.55.18` at `b9dd447a6`, which itself is past the storm fix in `0.55.17`),
packaged with `task package:macos`, installed from the resulting
`AgentMux_0.55.18_arm64.dmg`, and launched. The same macOS "wants to use
your confidential information stored in ... in your keychain" consent
dialog reappeared — the one believed fixed as of `0.55.16`/`0.55.17`.

Initial read of this as a code regression didn't hold up: diffing
`0.55.17..0.55.18` touches zero files under `*keychain*`, `*secret_store*`,
`*muxbus*`, or `identity/` — 0.55.18's only changes are fleet-control and
background-task-dashboard features. The actual regression is not in that
diff at all.

## 2. Investigation

- Confirmed which AgentMux instance was actually running on the machine at
  the time: `agentmux-srv-0.55.16-darwin.arm64`, from
  `channels/stable/versions/0.55.16` — i.e. an *older*, already-trusted
  install, launched before the 0.55.18 DMG was built. Its srv log
  (`agentmuxsrv-v0.55.16.log.2026-08-21`) shows ~930 lines of
  `auth.broker.fresh: credential is fresh` for `muxbus:global`, one per
  minute, all day — a live, working, already-granted credential, not a
  storm.
- The 0.55.18 srv log from the actual reported run
  (`agentmuxsrv-v0.55.18.log.2026-08-21`) has zero keychain/muxbus lines —
  the prompt happens at the OS level, outside the app's own log, so its
  absence there doesn't mean nothing happened.
- Dumped the real macOS Keychain: exactly **one** entry under service
  `"agentmux"`, account `acct:muxbus:global`, created `2026-08-20
  15:43:10Z`, most-recently read `2026-08-21 15:00:27Z` — that read
  timestamp lines up exactly with the last `auth.broker.fresh` line in the
  0.55.16 log. This confirms the storm fix from `0.55.17` is genuinely
  live and working on this machine: one blob, not twelve, being read
  successfully every minute by the app that already has consent for it.
- Compared code-signing identity between the already-running 0.55.16 app
  and the freshly built 0.55.18 app. The Keychain read itself runs in the
  `agentmux-srv` child binary (`Contents/MacOS/agentmux-srv-<version>-*`),
  which `package-macos.sh` signs *separately* from the outer `.app` — so
  that binary's own designated requirement, not the wrapping bundle's
  `CFBundleIdentifier`, is what macOS's Keychain ACL actually checks
  against. Comparing the two directly (`codesign -dr -` on each version's
  `agentmux-srv-*` executable, not the `.app`):
  ```
  0.54.12 srv: identifier "agentmux-srv-0.54.12-darwin" and anchor apple generic
               and ... certificate leaf[subject.OU] = "7Z3Z4B37QJ"
  0.55.16 srv: identifier "agentmux-srv-0.55.16-darwin" and anchor apple generic
               and ... certificate leaf[subject.OU] = "7Z3Z4B37QJ"
  ```
  Same Developer ID cert/team, but a **different `identifier`** per
  version — `package-macos.sh` signs this binary with `--entitlements`
  but no explicit `--identifier`, so codesign derives one from the
  binary's own versioned filename. (The outer `.app`'s `CFBundleIdentifier`
  is *also* version-baked on purpose — `ai.agentmux.<channel>.<version>`,
  "version suffix makes every release a distinct macOS app" — but it's not
  the identity this particular ACL check evaluates, since it's not the
  process making the call.)
- `agentmux-srv/src/identity/secret_store.rs` uses the `keyring` crate
  (`Entry::new(SERVICE, &account_key(account_id))`, `SERVICE = "agentmux"`)
  with no custom ACL/`kSecAttrAccessControl` — so macOS's default
  per-application access grant applies. That grant is scoped to the
  specific application identity that received it. A different designated
  requirement (different bundle id) is, to the OS, a different application
  — it has never been granted access to this Keychain item before,
  regardless of shared code-signing certificate.

## 3. Root cause

**Not a regression in the storm fix.** The `0.55.17` fix (collapse 12
entries to 1) is confirmed working: one blob, read successfully every
minute, no repeated prompting *within* the already-running, already-granted
0.55.16 process.

The recurrence is a **separate, previously-undiscussed interaction**
between two independent, individually-correct design decisions:

1. Every packaged build gets a **version-specific code identity** — the
   outer `.app`'s `CFBundleIdentifier` deliberately (`scripts/package-
   macos.sh`, so multiple local builds can coexist and each one launches
   cleanly without `open -n` gymnastics), and, as a side effect of how
   that same script signs the `agentmux-srv` child binary (entitlements
   but no explicit `--identifier`), the *actual Keychain-calling
   executable's* designated requirement too — derived from its own
   versioned filename.
2. macOS Keychain's default per-app access grant is scoped to the
   requesting *process's* code identity, i.e. `agentmux-srv`'s own
   designated requirement, not the wrapping `.app`'s — an "Always Allow"
   granted to `agentmux-srv-0.55.16-darwin` does not carry over to
   `agentmux-srv-0.55.18-darwin`, even though both are signed by the same
   Developer ID certificate, because the *identifier* clause differs.

Put together: **every distinct locally-built version is, to the Keychain,
a brand-new untrusted application**, so the very first read of the
existing `muxbus:global` blob from that new build re-triggers exactly one
fresh OS consent dialog — looking identical to the "storm" prompt, but
structurally unrelated to it. The 12→1 fix reduced *how many* prompts one
app identity could generate in a row; it did nothing to (and was never
scoped to) prevent a *new* app identity from needing its own first-time
grant. This is expected to recur on every single future local build,
release or not, for as long as bundle ids are version-specific and the
Keychain item's ACL isn't broadened past exact-app-identity.

## 4. What this is NOT

- Not the 12-entry storm (confirmed: exactly one entry exists).
- Not the unbounded-block hang from `retro-secret-store-keychain-read-
  timeout-2026-08-20.md` (the read succeeded — that's precisely how the
  Keychain item's `mdat` and the app log's timestamp could be correlated).
- Not caused by anything in the `0.55.17..0.55.18` diff — that diff never
  touches this code path.

## 5. Follow-up

- **Recommended, not yet built**: grant the Keychain item's ACL to any
  application signed by this Developer ID team, not just the exact
  requesting bundle id — e.g. build the item with an explicit
  `SecAccess`/trusted-application-list ACL keyed on
  `anchor apple generic and certificate leaf[subject.OU] = "7Z3Z4B37QJ"`
  instead of relying on the default per-exact-identity grant. This is the
  actual fix for "why does this keep happening on every new local build";
  everything in the three prior retros about entry count and read timeouts
  is orthogonal and already working correctly.
- If a team-wide ACL isn't desirable (e.g. wanting each version to be
  independently revocable), the alternative is documenting this as
  **expected, unavoidable behavior of the versioned-bundle-id scheme** —
  one one-time consent prompt per newly built version is then working as
  designed, and the actual bug report is "nobody had documented that yet."
  Given how often local builds happen on this machine (dozens of versions
  in `channels/stable/versions/` accumulated over months), the ACL fix is
  the more usable option.
- Worth checking whether other per-app-identity-scoped Keychain items in
  this codebase (the still-open `identity::secret_store` audit item from
  `retro-macos-keychain-saga-lessons-learned-2026-08-20.md` §3) have the
  same exposure — any item written without an explicit team-wide ACL will
  reprompt on every new bundle id, not just `muxbus:global`.
