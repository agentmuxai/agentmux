# Security Policy

## Supported Versions

We support the latest released version of AgentMux. Older versions may receive
security fixes at our discretion. Use the latest release for security-sensitive
deployments.

## Reporting a Vulnerability

**Do not** open public GitHub issues for security vulnerabilities.

Email: **security@agentmux.ai**

Please include:

- A description of the issue and its potential impact
- Steps to reproduce (proof-of-concept welcome)
- The version of AgentMux affected (`AgentMux --version` or About dialog)
- Your operating system and version
- Any suggested remediation

## Response Expectations

- **Acknowledgement:** within 3 business days
- **Initial assessment:** within 10 business days
- **Coordinated disclosure:** we follow a coordinated disclosure model. Please
  give us reasonable time to investigate and ship a fix before public disclosure.

## Scope

In scope:

- The AgentMux desktop application (`agentmux-cef`, `agentmux-launcher`)
- The bundled backend (`agentmux-srv`)
- Shell integration (`agentmux-bashwrap` crate and the `shell-integration/` scripts)
- Build and release tooling that produces shipped binaries

Out of scope:

- Vulnerabilities in upstream dependencies — please report to those projects;
  we will pick up the fix on the next release.
- Social engineering, physical attacks, or denial-of-service against
  AgentMux Corp. infrastructure
- Issues requiring physical access to an unlocked machine

## Credit

We will credit reporters in release notes (with permission) for valid findings.

## Disclaimer

This policy does not create any warranty obligation. AgentMux is provided
"AS IS" — see [LICENSE](./LICENSE) sections 7 and 8.
