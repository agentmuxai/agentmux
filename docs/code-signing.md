# AgentMux Code Signing Policy

AgentMux's Windows release binaries are code-signed through the free
**[SignPath Foundation](https://signpath.org/)** code-signing program for open-source
projects, using a certificate provided via **[SignPath.io](https://about.signpath.io/)**.

> Free code signing for AgentMux is provided by [SignPath.io](https://about.signpath.io/),
> with a certificate from the [SignPath Foundation](https://signpath.org/).

## What is signed

- The Windows installer — `AgentMux-<version>-x64-setup.exe`

Signed artifacts are published on the
[GitHub Releases](https://github.com/agentmuxai/agentmux/releases) page. Because the
certificate is issued under the SignPath Foundation OSS program, the signature's
**publisher is "SignPath Foundation"** (not "AgentMux Corp.").

## Team roles

Per the [SignPath Foundation conditions](https://signpath.org/terms.html), the project
maintains the roles below. All members use multi-factor authentication on both SignPath
and GitHub.

| Role | Responsibility | Members (GitHub) |
|------|----------------|------------------|
| **Author** | Trusted committer — writes and merges project code | [@a5af](https://github.com/a5af) |
| **Reviewer** | Reviews external contributions before they are merged | [@a5af](https://github.com/a5af) |
| **Approver** | Authorizes each signing request | [@a5af](https://github.com/a5af) |

## Build & signing process

1. Release artifacts are built from this public repository by CI — the private
   [`agentmux-builder`](https://github.com/agentmuxai/agentmux-builder) GitHub Actions
   pipeline (`build-windows.yml`) — from a specific release tag.
2. The unsigned installer is submitted to SignPath.io via the official
   [`signpath/github-action-submit-signing-request`](https://github.com/SignPath/github-action-submit-signing-request)
   action.
3. An **Approver** authorizes the signing request in SignPath.
4. SignPath signs the artifact in its HSM and returns the signed installer, which CI
   uploads to the GitHub Release.

The signing key is held exclusively in SignPath's HSM — the project never possesses it.
Only binaries built from this repository's source are signed.

## Privacy

This program processes the submitted build artifacts and the identities of the project's
signing team (for authorization and audit). It does **not** collect personal data from
end users of AgentMux. See the [SignPath privacy policy](https://about.signpath.io/privacy).

## Verifying a signature

```powershell
signtool verify /pa /v AgentMux-<version>-x64-setup.exe
```

The certificate chain should resolve to a certificate issued to **SignPath Foundation**.
