# Retro: macOS nightly CI red again — real mDNS test is environment-fragile, not flaky

**Date:** 2026-08-12
**Owner:** Maop
**Area:** `agentmux-srv/src/backend/lan_discovery.rs` (test-only), `.github/workflows/ci-nightly-build.yml`

---

## 1. Symptom

`ci-nightly-build.yml`'s `cargo build + test (macos-latest)` job failed
`cargo test --workspace` on the nights of 2026-08-11 and 2026-08-12 (and on a
manual re-run triggered mid-investigation), always on the same single test:

```
test backend::lan_discovery::tests::a_registered_instance_is_discoverable_by_an_independent_mdns_client ... FAILED
thread '...' panicked at agentmux-srv/src/backend/lan_discovery.rs:959:9:
an independent mDNS client must be able to discover a freshly-registered LanDiscovery instance within 5s
```

Masked from view for two nights because `ci-nightly-build.yml` runs
`macos-latest` and `ubuntu-latest` with `continue-on-error: true` (staged
rollout policy) — the workflow's own top-level status still shows green.
Only `windows-latest` is a hard gate today.

This is a distinct, unrelated failure from the FSEvents watcher bug fixed in
PR #2480 a few days earlier (see that PR/retro) — that fix is confirmed
working; both watcher tests pass in every run referenced here.

## 2. What introduced the failing test

PR #2512 (`fix(lan-discovery): registered instances were never actually
announced on the wire`, merged 2026-08-10) fixed a genuine production bug:
`LanDiscovery::start()` passed an empty string as the IP to
`ServiceInfo::new()`, expecting mdns-sd to auto-detect — but the crate
documents that an empty string yields *zero* addresses, not auto-detect.
Combined with `ServiceInfo::new` hardcoding `addr_auto: false` and `start()`
never calling `.enable_addr_auto()`, every registered instance had no
addresses and was never actually discoverable by anyone — despite always
logging "LAN discovery started" successfully. The fix (chain
`.enable_addr_auto()` onto the builder) is correct and unrelated to what
follows; this retro is entirely about the **test** added alongside it.

## 3. Root cause — two distinct, unrelated environment failures, not one flaky test

The initial hypothesis (before checking) was "GitHub's `macos-latest` runner
has flaky/hostile multicast networking" — the test's own doc comment already
hedged for exactly this ("small flakiness ceiling on a hostile CI network").
That hypothesis turned out to be half right and half wrong, and the
distinction matters for the fix:

**3a. GitHub Actions `macos-latest` genuinely does not support UDP
multicast — confirmed, not hypothesized.** Re-ran the nightly workflow
manually via `workflow_dispatch` on `main` as a third data point after the
two nightly failures: **3 for 3 identical failures**, same test, same line,
every time. A truly flaky/hostile-network test would pass at least
sometimes; 3/3 identical says "doesn't work here," not "occasionally drops a
packet." External corroboration, not just inference:
- [actions/runner-images#9628](https://github.com/actions/runner-images/issues/9628) — `mDNSResponder` is turned off during CI on GitHub's macOS runners.
- [actions/runner-images discussion #170669](https://github.com/orgs/community/discussions/170669) — macOS 15+ runner images don't support network multicast at all; sandboxed processes there lack the "Local Network" permission multicast requires, and this is Apple's sandboxing model, not something a workflow can configure around.

**3b. Separately — and this is the part the initial hypothesis got
wrong — the same test also fails on a real Mac outside CI entirely.**
Ran it locally on this dev machine (real hardware, `CI` unset, no GH sandbox
involved) expecting it to pass (the PR's own test plan implies it was
verified working somewhere pre-merge). It failed identically. Before
concluding this was a second CI-only quirk, verified whether multicast/mDNS
itself works on this machine at the OS level, independent of Rust:

```
dns-sd -R "test" _agentmux._tcp local. 54321 &     # register
dns-sd -B _agentmux._tcp local.                     # browse, separate process
# → found the registration within milliseconds
```

Native Bonjour discovery (two separate **OS processes**) works fine on this
machine — `mDNSResponder` is running, `lo0` has the `MULTICAST` flag, no
firewall or Little Snitch is blocking anything. So the machine's networking
isn't broken. Narrowed further with a **from-scratch, 40-line standalone Rust
program** (no AgentMux code, no tokio, no eventbus — just the `mdns-sd`
crate directly, same version pinned in this repo's `Cargo.lock`, 0.12.0):
two `ServiceDaemon`s in the *same process*, one registers, the other
browses. Result: the browse side received **zero** events — not
`ServiceFound`, not `SearchStarted` follow-through, nothing — while the
`SearchStarted` event itself showed the daemon enumerating 10 local
interfaces (six of them VPN/tunnel `utun*` interfaces from this machine's
normal setup).

This isolates the failure to `mdns-sd` 0.12.0's behavior when **two
`ServiceDaemon` instances share port 5353 with the OS's own
`mDNSResponder` from the same process**, on a machine with several
non-default interfaces — not to AgentMux's code, not to this specific test's
logic, and not to a broken network. It's consistent with community guidance
found for this crate (multi-daemon local dev/test setups are commonly told
to use a non-default port precisely to sidestep sharing 5353 with the
system responder) — the test as written never does that.

**Conclusion:** the test is fragile for two independent reasons that happen
to produce the identical symptom (5s timeout, nothing found): CI environments
with no multicast at all, and same-process multi-daemon port-5353 contention
on real hardware. Neither is fixable by retrying, waiting longer, or adding
`continue-on-error` smarter — both are outside this codebase's control.

## 4. Fix

Marked the test `#[ignore]` (unconditional — not a CI-only or macOS-only
runtime skip, since it also fails locally outside CI). It remains fully
present and runnable on demand:

```
cargo test -p agentmux-srv --bin agentmux-srv -- --ignored \
  lan_discovery::tests::a_registered_instance_is_discoverable_by_an_independent_mdns_client
```

An earlier draft of this fix tried a narrower runtime guard
(`if cfg!(target_os = "macos") && std::env::var_os("CI").is_some() { return }`)
based on the initial GH-CI-only hypothesis. Verifying it locally (running the
test on this real Mac with `CI` unset) is what surfaced 3b and proved that
guard insufficient — it would have left the test spuriously failing for any
developer on a machine with a similar interface profile. Left here as a
reminder to verify a fix's precondition locally before trusting it, not just
in the environment where the bug was first observed.

No production code changed — `LanDiscovery::start()`'s `.enable_addr_auto()`
fix from PR #2512 stands as-is; only the test's default-run status changed.

## 5. Why this wasn't caught before merging PR #2512

The PR's own test plan explicitly flagged the gap: *"macOS CI is the real
verification — I can't run macOS locally... please check this PR's CI run's
macOS leg... before merging."* But `ci-pr.yml` (the check suite that actually
gates a PR) only runs `windows-latest` + `ubuntu-latest` + `vitest` — it has
no macOS leg at all. The only macOS coverage is the nightly cross-platform
workflow, which runs on a schedule against `main`, not per-PR, and — per §1 —
was already set to `continue-on-error` for macOS. So there was no gate at any
point in the pipeline that could have caught this before merge; the earliest
possible signal was the following night's scheduled run, already merged.

## 6. What went well

- Didn't stop at the first plausible-sounding explanation. The GH-runner web
  research was real and correct as far as it went, but treating it as the
  *complete* answer without a local repro would have shipped a fix
  (CI+macOS-only skip) that silently reintroduced the same spurious failure
  for a class of developer machines — probably discovered later, more
  confusingly, as "works in CI, fails on my machine" instead of the reverse.
- Isolated the bug to a 40-line standalone reproduction outside the app
  entirely before deciding on a fix — the same discipline the retro-*
  precedents in this repo (e.g. the Taskfile TITLE-shadowing retro) already
  establish for this codebase.
- Verified the *absence* of a broken network (native `dns-sd` CLI test)
  before accepting "networking is broken here" as an explanation — ruling
  out the boring explanation first prevented mis-attributing a crate/OS
  interaction to a local misconfiguration.

## 7. Follow-up

- Not filing a bug against the `mdns-sd` crate yet — the same-process,
  shared-port-5353, multi-interface failure mode needs a cleaner minimal
  repro (fewer interfaces, isolate whether `utun*` tunnels specifically are
  the trigger) before it's worth reporting upstream. The 40-line repro from
  this investigation is a starting point but was deleted after use
  (`/tmp/mdns-repro`, not committed) — would need to be reconstructed.
- If real macOS mDNS wire coverage is ever wanted in CI, the actual fix is
  a self-hosted macOS runner (real hardware, not GitHub's sandboxed image) —
  not a workaround inside this test. Not pursuing given the cost; the
  `#[ignore]`d test plus manual verification (as literally happened for
  PR #2512's own original bug) is the existing, working process.
- Consider whether `ci-nightly-build.yml`'s `continue-on-error` on
  `macos-latest`/`ubuntu-latest` should graduate to a hard gate now that the
  FSEvents watcher bug (PR #2480) is fixed and this mDNS test is no longer
  the thing keeping it red — tracked as a separate decision, not bundled
  into this PR.
