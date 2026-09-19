#!/usr/bin/env python3
"""
github-app-token.py — mint a short-lived GitHub App installation access
token for a given agent identity, using the App ID / installation ID /
private key already provisioned in services/infra's agent-configs.<agent>
and <agent>-workflow-key entries.

Proof-of-concept / building block for retiring per-agent long-lived PATs
(gh-token-<agent>) in favor of GitHub App installation tokens, which expire
on their own (1 hour) and are minted fresh on every call - nothing to
rotate, nothing that can go stale in a git remote URL.

Usage:
    python3 scripts/github-app-token.py <agent-name-lowercase> <org>

<org> is the account that owns the target repo (e.g. "a5af" or
"agentmuxai") - an installation token is scoped to exactly one account, so
callers must say which one they need.

Prints ONLY the minted installation token to stdout (nothing else) so it
can be captured directly: TOKEN=$(python3 scripts/github-app-token.py agenty agentmuxai)
All diagnostic output goes to stderr.
"""
import shutil
import subprocess
import sys
import time

import jwt  # PyJWT
import requests


class NoAppIdentity(Exception):
    """Raised when an agent genuinely has no App identity provisioned - the
    caller should quietly fall back to another tier, not crash."""


class SecretsLookupError(Exception):
    """Raised when the secrets lookup itself failed (expired AWS credentials,
    no network, permission denied) as opposed to the value simply not being
    there. These two must not be conflated: 'this agent has no App' is a
    normal state worth falling through quietly, while 'I could not tell
    whether it has one' is a real fault that should be surfaced - otherwise
    an AWS outage silently downgrades the whole fleet to PATs and looks like
    business as usual."""


def get_secret_field(path):
    """Read one field from services/infra without ever printing the value.

    Goes through the `secrets` CLI (same as scripts/gh-agent.sh), not raw
    `aws secretsmanager` calls - this needs to work under whichever scoped
    IAM role/profile the calling agent actually has (agent-configs.<agent>
    .aws_profile), not just a session with broad root-account credentials.
    """
    # Python's subprocess (no shell) can't resolve the extensionless `secrets`
    # shell shim the way bash does (WinError 2) - it needs the .cmd variant
    # explicitly on Windows. shutil.which("secrets") follows PATHEXT and
    # finds secrets.cmd; falls back to the bare name on non-Windows.
    secrets_bin = shutil.which("secrets") or "secrets"
    proc = subprocess.run(
        [secrets_bin, "get", "services/infra", "--path", path, "--raw", "--no-warning"],
        capture_output=True, text=True,
    )
    # stdout carries the SECRET VALUE itself (that is what --raw means), so it
    # is matched against but NEVER echoed. stderr is the only stream safe to
    # surface. This distinction is load-bearing: when this function is called
    # for `<agent>-workflow-key`, stdout is a private RSA key, and an error
    # message built from stdout would leak it into logs and terminals via
    # gh-agent.sh, which prints this script's stderr verbatim.
    out = proc.stdout or ""
    err = proc.stderr or ""

    # The CLI says so explicitly when the path simply isn't there. That is the
    # ONLY case that means "this agent has no App identity" - everything else
    # non-zero means the lookup itself broke, which must not be reported as
    # "not provisioned" or an AWS outage silently downgrades the whole fleet
    # to PATs while looking like business as usual.
    if "Path not found" in out or "Path not found" in err:
        raise NoAppIdentity(f"no value at services/infra:{path}")

    if proc.returncode != 0:
        raise SecretsLookupError(
            f"secrets lookup failed for services/infra:{path} "
            f"(exit {proc.returncode}): {err.strip()[:300]}"
        )

    if not proc.stdout.strip():
        raise NoAppIdentity(f"no value at services/infra:{path}")

    return proc.stdout.rstrip("\n")


def get_installation_id(agent_name, org):
    """Resolve the installation ID for a given org.

    Agents whose App is installed on more than one account (e.g. both a5af
    and agentmuxai) store github_app_installation_id as a per-org map
    instead of a single scalar - a token minted from one org's installation
    has zero access to the other org's repos. Try the per-org path first,
    then fall back to the old flat scalar for agents that only ever had one
    installation and were never migrated to the map form.
    """
    try:
        return get_secret_field(f"agent-configs.{agent_name}.github_app_installation_id.{org}")
    except NoAppIdentity:
        return get_secret_field(f"agent-configs.{agent_name}.github_app_installation_id")


def mint_installation_token(agent_name, org):
    app_id = get_secret_field(f"agent-configs.{agent_name}.github_app_id")
    installation_id = get_installation_id(agent_name, org)
    private_key = get_secret_field(f"{agent_name}-workflow-key")

    now = int(time.time())
    payload = {
        "iat": now,
        # GitHub enforces exp - iat <= 600s exactly, not exp - actual_now.
        # A backdated iat (common clock-drift-buffer pattern elsewhere) plus
        # a full 600s here overshoots that and is rejected with "Expiration
        # time claim ('exp') is too far in the future" - confirmed by testing.
        "exp": now + 540,
        "iss": str(app_id),
    }
    app_jwt = jwt.encode(payload, private_key, algorithm="RS256")

    resp = requests.post(
        f"https://api.github.com/app/installations/{installation_id}/access_tokens",
        headers={
            "Authorization": f"Bearer {app_jwt}",
            "Accept": "application/vnd.github+json",
        },
        timeout=15,
    )
    resp.raise_for_status()
    body = resp.json()
    print(f"[github-app-token] minted for {agent_name}, expires_at={body['expires_at']}, "
          f"repos={'all' if 'repositories' not in body else len(body.get('repositories', []))}",
          file=sys.stderr)
    return body["token"]


if __name__ == "__main__":
    if len(sys.argv) != 3:
        print("usage: github-app-token.py <agent-name-lowercase> <org>", file=sys.stderr)
        sys.exit(1)
    try:
        token = mint_installation_token(sys.argv[1], sys.argv[2])
    except NoAppIdentity as e:
        # Distinct exit code so gh-agent.sh can tell "no App identity, fall
        # through quietly" apart from "something actually broke" (exit 1).
        # gh-agent.sh branches on this code, not on this message's text.
        print(f"[github-app-token] {e} - no App identity for this agent", file=sys.stderr)
        sys.exit(2)
    except SecretsLookupError as e:
        print(f"[github-app-token] {e}", file=sys.stderr)
        sys.exit(1)
    except Exception as e:
        print(f"[github-app-token] failed to mint token: {e}", file=sys.stderr)
        sys.exit(1)
    print(token)
