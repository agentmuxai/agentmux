# SPEC — Own the `cef-dll-sys` binding fork: `agentmuxai/cef-rs`

**Date:** 2026-09-22
**Status:** implemented — fork created and populated 2026-09-22, workspace repointed in this PR
**Author:** AgentY
**Related:**
`Cargo.toml` (`[patch.crates-io]`, the single pin this spec is about),
`docs/cef-build/CEF_FORK_MAINTENANCE.md` (the *libcef* fork's carry-set
practice — a different repo, `agentmuxai/cef`, already org-owned and already
CI-gated by `scripts/cef-verify.sh`),
`docs/specs/SPEC_CEF_MILESTONE_UPGRADE_148_TO_152_2026_09_07.md` (§Phase C
is where the pin first moved to a personal fork "pending" an upstream PR),
`docs/specs/SPEC_CEF_148_LINUX_FORWARD_PORT_2026_06_04.md` §5 (why the
binding needs a patch at all)

---

## 1. Why this spec exists

AgentMux's Chromium layer depends on two forks, and until today only one of
them was owned by the org:

| Layer | Upstream | Our fork | Owner before | Owner now |
|---|---|---|---|---|
| libcef (C++) | `chromiumembedded/cef` | `agentmuxai/cef` | org | org |
| Rust binding (`cef-dll-sys`) | `tauri-apps/cef-rs` | `agentmuxai/cef-rs` | **two personal accounts** | org |

Every build of `agentmux-cef` — every developer's `task dev`, every CI
release on all three platforms — resolves `cef-dll-sys` from the git URL in
the root `Cargo.toml`'s `[patch.crates-io]`. That URL pointed at
`GenericAgentX-asaf/cef-rs`, which is itself a fork of `AgentU-asaf/cef-rs`,
which is the fork of `tauri-apps/cef-rs`. Two personal GitHub accounts, one
of them an agent identity, stood between the org and its ability to build
its own desktop app. If either account were deleted, suspended, renamed, or
had the fork removed, `cargo` would fail at dependency resolution with no
fallback — the pinned rev exists nowhere else.

This is not hypothetical exposure. The pin already moved once
(`AgentU-asaf` → `GenericAgentX-asaf`, SPEC_CEF_MILESTONE_UPGRADE §Phase C)
precisely because the first account had no way to merge its own PRs. The
`Cargo.toml` comment said "repoint once either upstream PR lands"; those PRs
(`AgentU-asaf/cef-rs#4`, `#5`) have been open since 2026-09-11 with nobody
able to merge them. Waiting on them was never going to resolve this.

## 2. What this changes

1. **`agentmuxai/cef-rs` exists**, created 2026-09-22 as a GitHub fork of
   `tauri-apps/cef-rs` (so it keeps upstream lineage and can pull future
   binding releases the normal way), and populated with all four
   `agentmux/*` branches from `GenericAgentX-asaf/cef-rs`:

   | Branch | Tip | Carries |
   |---|---|---|
   | `agentmux/148-begin-window-drag` | (148 era) | `begin_window_drag` slot, linux_x86_64, CEF 148 |
   | `agentmux/fix-links-collision-148` | | + `links` rename, 148 |
   | `agentmux/152-begin-window-drag` | `bfeae804` | `begin_window_drag` slot, CEF 152 |
   | `agentmux/fix-links-collision-152` | `9b0abfe6` | + `links` rename, 152 — **the pinned rev** |

   These were pushed as git refs, not re-committed, so every commit SHA is
   identical to the one under the personal forks. That is why step 2 is a
   one-word change.

2. **The `[patch.crates-io]` URL is now `https://github.com/agentmuxai/cef-rs`**
   at the same `rev = "9b0abfe6…"`. `Cargo.lock` changes owner on exactly
   two `source` lines — `cef-dll-sys` and `download-cef`, which the same
   repo ships — and nothing else; a git source is identified by rev, and
   the rev's content is unchanged.

3. The two live build docs under `docs/cef-build/` that told a developer to
   fetch the binding from `AgentU-asaf` now name the org fork. Historical
   specs, reports, and retros that mention the personal forks are left as
   written — they record what was true when they were written.

## 3. What this deliberately does not change

- **The pinned commit.** Repointing and re-pinning in one PR would make it
  impossible to tell a resolution failure from a binding regression. The
  content is byte-identical; only the host moved.
- **The branch layout in the fork.** The `fix-links-collision-*` branches
  sit one commit above their `*-begin-window-drag` bases, mirroring the
  unmerged upstream PR stack. Collapsing that into one `agentmux/152`
  integration branch (the model `CEF_FORK_MAINTENANCE.md` §4 uses for the
  libcef fork) is a reasonable follow-up, but it changes SHAs and belongs
  in its own PR with its own re-pin.
- **The upstream PRs.** `AgentU-asaf/cef-rs#4`/`#5` can stay open or be
  closed; nothing in this repo depends on them any more. The right
  long-term move is a PR against `tauri-apps/cef-rs` itself, from the org
  fork, which is now possible.
- **The libcef fork.** `agentmuxai/cef` and the carry-set gate
  (`scripts/cef-verify.sh`, run on every PR by `ci-pr.yml`) are the
  answer to the *other* CEF risk — silently losing patches across a
  milestone upgrade — and were already in place. This spec closes the
  ownership gap on the binding side only.

## 4. Verification

- `cargo metadata` re-resolves `cef-dll-sys` and `download-cef` from the
  org URL at the same rev; the `Cargo.lock` diff is exactly those two
  `source` lines. (Observed in this PR.)
- `gh repo view agentmuxai/cef-rs` reports `isFork: true`, parent
  `tauri-apps/cef-rs`; `gh api repos/agentmuxai/cef-rs/branches` lists the
  four `agentmux/*` branches; the pinned rev resolves there.
- The first CI run on this PR builds `agentmux-cef` on all three platforms
  from the org URL — that build *is* the proof the resolution works outside
  one developer's git cache.

## 5. Residual risk, stated plainly

The binding is now as durable as the libcef fork: it lives under the org
and any org admin can maintain it. What remains is the same thing that
remains for `agentmuxai/cef` — the patch is carried by hand across
`cef-dll-sys` releases, and a future bump of the `cef` crate in
`agentmux-cef/Cargo.toml` must be accompanied by re-porting the
`begin_window_drag` slot and the `links` rename onto the matching binding
version. There is no equivalent of `cef-verify.sh` for the binding; a
`--features patched-libcef` build failing with `no field begin_window_drag`
is currently the detection mechanism, and it is a loud one.
