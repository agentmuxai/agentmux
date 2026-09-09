# Windows Code Signing — research, decision & setup (DEFERRED)

> **Provenance note:** this document was written 2026-06 in the `agentmux-builder`
> repo, which no longer exists on GitHub (confirmed gone from the `agentmuxai` org
> under any name, 2026-09-09 — not access-restricted, actually deleted). Recovered
> from a stale local clone and landed here since it's the only surviving copy and
> the research/decision themselves are still fully valid. One factual claim from
> the original has been corrected below: it said the CI plumbing was "already
> built and inert" — that plumbing lived only in `agentmux-builder` and does not
> exist in this repo's `build-windows.yml`, which currently ships an **unsigned**
> MSIX with no signing step at all.

> **Status (2026-06): DEFERRED.** AgentMux does not yet code-sign Windows artifacts. The
> project's reputation is too new for a signature to meaningfully help — Microsoft
> SmartScreen reputation accrues with download volume over weeks, and the EV "instant
> clean" bypass was removed in 2024, so a brand-new signature warns just like an unsigned
> one until reputation builds. The installer ships **unsigned** for now (SmartScreen:
> *More info → Run anyway*). This doc preserves the research + chosen path so we can flip
> it on when we circle back.

## TL;DR

- **Chosen path (when we activate): SignPath Foundation** — free code signing for Apache-2.0 OSS.
- **Not EV.** EV no longer bypasses SmartScreen (removed 2024) and is only needed for kernel drivers (we have none). OV-level is functionally equivalent now — don't pay the EV premium or deal with USB tokens.
- **The CI plumbing does not exist yet in this repo.** Activating this means adding the signing step to `.github/workflows/build-windows.yml` from scratch, not flipping on something already wired — see the corrected checklist below.

## The 2026 landscape (why these choices)

- **EV is dead for SmartScreen.** Microsoft removed EV's instant-reputation bypass in 2024; EV-signed apps now build reputation exactly like OV. EV only matters for kernel-mode drivers.
- **Reputation accrues over time** regardless of cert type — any trusted OV signature clears the "unknown publisher" warning *once enough downloads accrue*. (This is the core reason to defer: signing a download-history-less app doesn't instantly help.)
- **OV keys must live in hardware/cloud** (since June 2023) — no more file-based `.pfx`; you use a cloud-HSM service or a managed program.
- **Apache-2.0 OSS unlocks free signing** via SignPath Foundation / OSSign.

### Options compared

| Path | Cost | Publisher shown | CI fit |
|---|---|---|---|
| **SignPath Foundation** (chosen) | Free | "SignPath Foundation" | Managed GitHub Action |
| Azure Artifact Signing (ex-"Trusted Signing") | Free now (paid "soon") | Your own "AgentMux Corp." | Native (signtool + Azure dlib) |
| Commercial OV (SSL.com eSigner, Certum) | ~$220/yr | Your own | Cloud-HSM API |

Sources: [MS Learn — code signing options](https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/code-signing-options) · [Azure Artifact Signing](https://azure.microsoft.com/en-us/products/artifact-signing) · [SignPath Foundation terms](https://signpath.org/terms.html) · [EV vs OV / SmartScreen 2024](https://sslinsights.com/ov-code-signing-vs-ev-code-signing-certificate/) · [Certum OSS](https://certum.store/open-source-code-signing-code.html)

## How SignPath Foundation works

- The signing key lives in **SignPath's HSM** — the project never holds a cert file.
- **CI** builds the artifact → submits it to SignPath via their GitHub Action → a human **Approver** authorizes → SignPath returns the signed binary → CI uploads it to the release.
- Trade-off accepted: the signed **publisher shows "SignPath Foundation"**, not "AgentMux Corp."

## Activation checklist (when we circle back)

1. **Apply** at [signpath.org → Open Source](https://signpath.org/). Requirements: OSI license (Apache-2.0 ✓), no commercial dual-licensing, no proprietary code, actively maintained + already released ✓, a **published code-signing policy** (template below), and team roles (Author/Reviewer/Approver) with **MFA** on SignPath + GitHub.
2. In **SignPath.io**: create the `agentmux` project + a signing policy (slug e.g. `release-signing`); set Author/Reviewer/Approver = **a5af**; enable MFA.
3. Add to **this repo**: secrets `SIGNPATH_API_TOKEN`, `SIGNPATH_ORG_ID`; repo variable `SIGNPATH_POLICY_SLUG`.
4. **Publish** the code-signing policy publicly (e.g. `agentmux/docs/code-signing.md`, using the template below).
5. **Write the signing step into `build-windows.yml`** (it does not exist yet — see the TL;DR correction above), then `workflow_dispatch` to validate on a runner before it runs on a real release.

## Code-signing policy — template to publish publicly when we go live

> Drafted with **@a5af** as Author/Reviewer/Approver. Publish at a public URL (e.g.
> `agentmux/docs/code-signing.md`) — SignPath Foundation requires a public policy page.

```markdown
# AgentMux Code Signing Policy

AgentMux's Windows release binaries are code-signed through the free
[SignPath Foundation](https://signpath.org/) program for open-source projects, using a
certificate provided via [SignPath.io](https://about.signpath.io/).

> Free code signing for AgentMux is provided by [SignPath.io](https://about.signpath.io/),
> with a certificate from the [SignPath Foundation](https://signpath.org/).

## What is signed
- The Windows installer — `AgentMux-<version>-x64-setup.exe`. The signature's publisher
  is "SignPath Foundation" (the OSS-program certificate holder).

## Team roles (all with MFA on SignPath + GitHub)
| Role | Responsibility | Members |
|------|----------------|---------|
| Author   | Trusted committer | @a5af |
| Reviewer | Reviews external contributions | @a5af |
| Approver | Authorizes each signing request | @a5af |

## Build & signing process
1. CI (`build-windows.yml`, this repo) builds the installer from a release tag.
2. The unsigned installer is submitted to SignPath.io via `signpath/github-action-submit-signing-request`.
3. An Approver authorizes the signing request.
4. SignPath signs in its HSM and CI uploads the signed installer to the GitHub Release.

The signing key never leaves SignPath's HSM; only binaries built from this repo are signed.

## Privacy
Processes build artifacts + the signing team's identities (authorization/audit); collects
no end-user data. See the [SignPath privacy policy](https://about.signpath.io/privacy).

## Verifying
`signtool verify /pa /v AgentMux-<version>-x64-setup.exe` → chain resolves to SignPath Foundation.
```

## SmartScreen expectation (set this when we go live)

Even once signing is on, the "unknown publisher" warning persists until SmartScreen
reputation accrues (downloads over weeks). EV would **not** shortcut this anymore (changed
2024). Plan for a reputation ramp, not an instant fix — which is exactly why this is
deferred until AgentMux has download volume worth building reputation on.
