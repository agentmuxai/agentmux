#!/usr/bin/env bash
# Build the signed apt and dnf/zypper repositories for AgentMux's .deb and .rpm packages.
#
# Repo-1 of docs/specs/SPEC_LINUX_PACKAGE_UPGRADE_AND_REPOSITORY_2026_10_24.md §4. This builds the
# tree to upload; it uploads nothing, and holds no key (the caller provides one).
#
# Usage:
#   GPG_KEY=<fingerprint> scripts/build-linux-repo.sh <repo-dir>
#   SIGNING_KEY_FILE=<armored secret subkey> scripts/build-linux-repo.sh <repo-dir>
#
# <repo-dir> is the whole repository, as it is (or will be) on the origin. The caller first copies the
# origin's current packages into it (so earlier versions stay) and adds the new ones:
#   <repo-dir>/apt/pool/main/a/agentmux/*.deb
#   <repo-dir>/rpm/x86_64/*.rpm
# This script then writes, next to them:
#   <repo-dir>/apt/dists/<suite>/{InRelease,Release,Release.gpg}, main/binary-amd64/Packages{,.xz} and by-hash
#   <repo-dir>/rpm/x86_64/repodata/ (with repomd.xml.asc)
#   <repo-dir>/agentmux.asc, agentmux-archive-keyring.gpg   the public key (armored; binary, for apt's Signed-By)
#   <repo-dir>/agentmux.sources, agentmux.repo              what a client installs to subscribe
# and signs every .rpm that is not signed yet. A .deb is not signed itself: apt checks the signed index.
#
# Upload order (the caller's job, see the spec's §4.5): packages first, then the indices; Release/InRelease
# and repomd.xml last, so a client never reads an index that lists a file the origin doesn't have yet.
#
# Environment (one of the first two):
#   GPG_KEY              the signing key's fingerprint or id, in GNUPGHOME's keyring
#   SIGNING_KEY_FILE     an ASCII-armored export of the signing SUBKEY (`gpg --export-secret-subkeys --armor`), as the
#                        release pipeline holds it: imported into a keyring of its own, which is deleted at the end,
#                        and the subkey signs (the primary key is never here)
#   GNUPGHOME            the keyring for GPG_KEY (default ~/.gnupg)
#   GPG_PASSPHRASE_FILE  a file holding the key's passphrase, when it has one
#   REPO_URL             where the repository is served (default https://packages.agentmux.ai); only written into
#                        the client files
#   SUITE                the apt suite (default stable)
#
# Needs: gpg, apt-ftparchive (apt-utils), xz, createrepo_c, rpm (rpmsign), dpkg-deb, sha256sum.
set -euo pipefail

REPO="${1:?usage: GPG_KEY=<fingerprint> build-linux-repo.sh <repo-dir>}"
REPO_URL="${REPO_URL:-https://packages.agentmux.ai}"
REPO_URL="${REPO_URL%/}"
SUITE="${SUITE:-stable}"

for tool in gpg apt-ftparchive xz createrepo_c rpmsign rpm dpkg-deb sha256sum; do
    command -v "$tool" >/dev/null 2>&1 || { echo "ERROR: $tool not found on PATH" >&2; exit 1; }
done
[ -d "$REPO" ] || { echo "ERROR: $REPO is not a directory" >&2; exit 1; }

if [ -n "${SIGNING_KEY_FILE:-}" ]; then
    GNUPGHOME="$(mktemp -d)"
    export GNUPGHOME
    trap 'gpgconf --kill gpg-agent >/dev/null 2>&1 || true; rm -rf "$GNUPGHOME"' EXIT
    gpg --batch --quiet --import "$SIGNING_KEY_FILE" 2>/dev/null
    # The first secret subkey that can sign; the "!" makes gpg use exactly that one.
    sub="$(gpg --batch --list-secret-keys --with-colons | awk -F: '$1 == "ssb" && $12 ~ /s/ { want = 1; next } want && $1 == "fpr" { print $10; exit }')"
    [ -n "$sub" ] || { echo "ERROR: $SIGNING_KEY_FILE holds no secret signing subkey" >&2; exit 1; }
    GPG_KEY="$sub!"
fi
: "${GPG_KEY:?set GPG_KEY or SIGNING_KEY_FILE}"
# The public key to publish is the whole key the signing key belongs to.
PUBLIC_KEY="${GPG_KEY%!}"
REPO="$(cd "$REPO" && pwd)"

POOL="$REPO/apt/pool/main/a/agentmux"
RPMDIR="$REPO/rpm/x86_64"
DIST="$REPO/apt/dists/$SUITE"
mkdir -p "$POOL" "$RPMDIR"

shopt -s nullglob
debs=("$POOL"/*.deb)
rpms=("$RPMDIR"/*.rpm)
[ "${#debs[@]}" -gt 0 ] || { echo "ERROR: no .deb under $POOL" >&2; exit 1; }
[ "${#rpms[@]}" -gt 0 ] || { echo "ERROR: no .rpm under $RPMDIR" >&2; exit 1; }

# The signing key is the only thing the sign commands take from the environment.
GPG_ARGS=(--batch --yes --pinentry-mode loopback --local-user "$GPG_KEY")
RPM_ARGS=(--define "_gpg_name $GPG_KEY" --define "__gpg $(command -v gpg)")
if [ -n "${GPG_PASSPHRASE_FILE:-}" ]; then
    GPG_ARGS+=(--passphrase-file "$GPG_PASSPHRASE_FILE")
    RPM_ARGS+=(--define "_gpg_sign_cmd_extra_args --pinentry-mode loopback --passphrase-file $GPG_PASSPHRASE_FILE")
fi
sign_detached() { gpg "${GPG_ARGS[@]}" --armor --detach-sign --output "$2" "$1"; }

# --- 1. The public key, in the two forms clients want. ---
gpg --batch --yes --armor --export "$PUBLIC_KEY" >"$REPO/agentmux.asc"
[ -s "$REPO/agentmux.asc" ] || { echo "ERROR: no public key for $PUBLIC_KEY in the keyring" >&2; exit 1; }
gpg --batch --yes --export "$PUBLIC_KEY" >"$REPO/agentmux-archive-keyring.gpg"

# --- 2. Sign the .rpm files that are not signed yet (a signed one keeps its signature). ---
for f in "${rpms[@]}"; do
    if [ "$(rpm -qp --queryformat '%{RSAHEADER:pgpsig}' "$f" 2>/dev/null)" = "(none)" ]; then
        rpmsign --addsign "${RPM_ARGS[@]}" "$f" >/dev/null
        echo "signed $(basename "$f")"
    fi
done

# --- 3. apt: Packages, Release, InRelease, Release.gpg, by-hash. ---
BIN="$DIST/main/binary-amd64"
rm -rf "$DIST"
mkdir -p "$BIN"
(
    cd "$REPO/apt"
    # Paths in Packages are relative to the apt root, as apt's Filename: wants.
    apt-ftparchive packages pool >"dists/$SUITE/main/binary-amd64/Packages"
)
xz -9 -k -f "$BIN/Packages"
# by-hash keeps a client that reads Release and then the indices from seeing two different publishes.
for f in "$BIN/Packages" "$BIN/Packages.xz"; do
    sum="$(sha256sum "$f" | cut -d' ' -f1)"
    mkdir -p "$BIN/by-hash/SHA256"
    cp "$f" "$BIN/by-hash/SHA256/$sum"
done
(
    cd "$DIST"
    apt-ftparchive \
        -o "APT::FTPArchive::Release::Origin=AgentMux" \
        -o "APT::FTPArchive::Release::Label=AgentMux" \
        -o "APT::FTPArchive::Release::Suite=$SUITE" \
        -o "APT::FTPArchive::Release::Codename=$SUITE" \
        -o "APT::FTPArchive::Release::Architectures=amd64" \
        -o "APT::FTPArchive::Release::Components=main" \
        -o "APT::FTPArchive::Release::Acquire-By-Hash=yes" \
        release . >Release
)
gpg "${GPG_ARGS[@]}" --clearsign --output "$DIST/InRelease" "$DIST/Release"
gpg "${GPG_ARGS[@]}" --armor --detach-sign --output "$DIST/Release.gpg" "$DIST/Release"

# --- 4. dnf and zypper: repodata, signed. ---
rm -rf "$RPMDIR/repodata"
createrepo_c --quiet --general-compress-type xz "$RPMDIR"
sign_detached "$RPMDIR/repodata/repomd.xml" "$RPMDIR/repodata/repomd.xml.asc"

# --- 5. What a client installs to subscribe (§4.7 puts these inside the packages). ---
cat >"$REPO/agentmux.sources" <<EOF
Types: deb
URIs: $REPO_URL/apt
Suites: $SUITE
Components: main
Architectures: amd64
Signed-By: /usr/share/keyrings/agentmux-archive-keyring.gpg
EOF
cat >"$REPO/agentmux.repo" <<EOF
[agentmux]
name=AgentMux
baseurl=$REPO_URL/rpm/x86_64
enabled=1
gpgcheck=1
repo_gpgcheck=1
gpgkey=$REPO_URL/agentmux.asc
EOF

echo "repository built in $REPO: ${#debs[@]} .deb, ${#rpms[@]} .rpm, suite $SUITE"
