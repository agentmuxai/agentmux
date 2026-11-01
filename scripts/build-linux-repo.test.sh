#!/usr/bin/env bash
# Test scripts/build-linux-repo.sh in containers, with a throwaway key and stand-in packages.
#
#   scripts/build-linux-repo.test.sh
#
# Needs Docker. Builds a repository from two versions of a tiny `agentmux` .deb and .rpm, then checks that
# real apt (Ubuntu 24.04), dnf (Fedora 42) and zypper (openSUSE Tumbleweed) install from it with the signature
# checks on, that they refuse a tampered index, and that an upgrade finds the newer version.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
mkdir -p "$WORK/repo"
cp "$HERE/build-linux-repo.sh" "$WORK/"

# MSYS_NO_PATHCONV: Git Bash must not rewrite the container paths below.
export MSYS_NO_PATHCONV=1
# Docker on Windows wants a Windows path for a bind mount.
MOUNT="$WORK"
if command -v cygpath >/dev/null 2>&1; then MOUNT="$(cygpath -m "$WORK")"; fi

echo "== build the repository (Ubuntu 24.04) =="
docker run --rm -v "$MOUNT:/w" ubuntu:24.04 bash -euc '
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq >/dev/null
apt-get install -y -qq --no-install-recommends gnupg apt-utils xz-utils createrepo-c rpm ca-certificates >/dev/null

# The pipeline way: a primary key (certify only) and a signing subkey with a passphrase; only the subkey is exported.
export GNUPGHOME=/tmp/gnupg-owner; mkdir -p "$GNUPGHOME"; chmod 700 "$GNUPGHOME"
gpg --batch --pinentry-mode loopback --passphrase test-pass --quick-gen-key "AgentMux test key <test@example.invalid>" rsa4096 cert never 2>/dev/null
KEY="$(gpg --batch --list-keys --with-colons | awk -F: "/^fpr/ {print \$10; exit}")"
gpg --batch --pinentry-mode loopback --passphrase test-pass --quick-add-key "$KEY" rsa4096 sign 2y 2>/dev/null
gpg --batch --pinentry-mode loopback --passphrase test-pass --export-secret-subkeys --armor "$KEY" >/w/subkey.asc
printf test-pass >/w/pass.txt
# From here on the build sees only the exported subkey, in a keyring of its own.
unset GNUPGHOME

mkdir -p /w/repo/apt/pool/main/a/agentmux /w/repo/rpm/x86_64
for v in 1.0.0 1.1.0; do
    d=/tmp/deb-$v; mkdir -p $d/DEBIAN $d/opt/agentmux
    printf "Package: agentmux\nVersion: $v\nArchitecture: amd64\nMaintainer: t <t@example.invalid>\nDescription: stand-in\n" >$d/DEBIAN/control
    echo $v >$d/opt/agentmux/version
    dpkg-deb --build $d /w/repo/apt/pool/main/a/agentmux/AgentMux_${v}_amd64.deb >/dev/null

    r=/tmp/rpm-$v; mkdir -p $r/{SPECS,BUILD,RPMS,SOURCES}
    printf "Name: agentmux\nVersion: $v\nRelease: 1\nSummary: stand-in\nLicense: Apache-2.0\nBuildArch: x86_64\n%%description\nstand-in\n%%install\nmkdir -p %%{buildroot}/opt/agentmux\necho $v > %%{buildroot}/opt/agentmux/version\n%%files\n/opt/agentmux/version\n" >$r/SPECS/a.spec
    rpmbuild --define "_topdir $r" -bb $r/SPECS/a.spec >/dev/null 2>&1
    cp $r/RPMS/x86_64/*.rpm /w/repo/rpm/x86_64/
done

SIGNING_KEY_FILE=/w/subkey.asc GPG_PASSPHRASE_FILE=/w/pass.txt REPO_URL=https://packages.example.invalid /w/build-linux-repo.sh /w/repo
# A second run changes nothing it should not: signed rpms stay signed, the index is rebuilt.
SIGNING_KEY_FILE=/w/subkey.asc GPG_PASSPHRASE_FILE=/w/pass.txt REPO_URL=https://packages.example.invalid /w/build-linux-repo.sh /w/repo | tail -1
# the secret key never reached the build, and the published key has the primary and the signing subkey
ls /tmp/gnupg-owner >/dev/null
gpg --batch --show-keys --with-colons /w/repo/agentmux.asc | awk -F: "\$1==\"sub\" && \$12 ~ /s/ {found=1} END {exit !found}"
gpg --batch --show-keys --with-colons /w/repo/agentmux.asc | grep -q "^sec" && { echo "PUBLISHED A SECRET KEY"; exit 1; } || true
export GNUPGHOME=/tmp/gnupg-verify; mkdir -p $GNUPGHOME; chmod 700 $GNUPGHOME
gpg --batch --quiet --import /w/repo/agentmux.asc
gpg --batch --yes --verify /w/repo/apt/dists/stable/InRelease
for f in /w/repo/rpm/x86_64/*.rpm; do
    [ "$(rpm -qp --queryformat "%{RSAHEADER:pgpsig}" $f)" != "(none)" ] || { echo "UNSIGNED $f"; exit 1; }
done
echo "$KEY" >/w/key.fpr
'

echo "== apt (Ubuntu 24.04): install 1.0.0, upgrade to 1.1.0, refuse a tampered index =="
docker run --rm -v "$MOUNT:/w:ro" ubuntu:24.04 bash -euc '
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq >/dev/null; apt-get install -y -qq --no-install-recommends ca-certificates >/dev/null
rm -rf /srv/repo; mkdir -p /srv; cp -r /w/repo /srv/repo
install -m 644 /srv/repo/agentmux-archive-keyring.gpg /usr/share/keyrings/agentmux-archive-keyring.gpg
sed "s#^URIs:.*#URIs: file:///srv/repo/apt#" /srv/repo/agentmux.sources >/etc/apt/sources.list.d/agentmux.sources
grep -q "^Signed-By: /usr/share/keyrings/agentmux-archive-keyring.gpg" /etc/apt/sources.list.d/agentmux.sources
apt-get update 2>&1 | tail -3
apt-get install -y -qq agentmux=1.0.0 >/dev/null; [ "$(cat /opt/agentmux/version)" = 1.0.0 ]
apt-get install -y -qq --only-upgrade agentmux >/dev/null; [ "$(cat /opt/agentmux/version)" = 1.1.0 ]
echo "apt installs and upgrades from the repository"
# a tampered Release: the signature check must fail the update
sed -i "s/^Label: AgentMux/Label: Evil/" /srv/repo/apt/dists/stable/Release /srv/repo/apt/dists/stable/InRelease
rm -rf /var/lib/apt/lists/*
if apt-get update 2>&1 | grep -qiE "not signed|signature|BADSIG|NO_PUBKEY|Release.*(error|failed)"; then echo "apt refuses a tampered index"; else echo "apt accepted a tampered index"; exit 1; fi
'

echo "== dnf (Fedora 42): install 1.0.0, upgrade, refuse a tampered repomd =="
docker run --rm -v "$MOUNT:/w:ro" fedora:42 bash -euc '
rm -rf /srv/repo; mkdir -p /srv; cp -r /w/repo /srv/repo
sed -e "s#^baseurl=.*#baseurl=file:///srv/repo/rpm/x86_64#" -e "s#^gpgkey=.*#gpgkey=file:///srv/repo/agentmux.asc#" /srv/repo/agentmux.repo >/etc/yum.repos.d/agentmux.repo
dnf install -y -q agentmux-1.0.0 >/dev/null; [ "$(cat /opt/agentmux/version)" = 1.0.0 ]
dnf upgrade -y -q agentmux >/dev/null; [ "$(cat /opt/agentmux/version)" = 1.1.0 ]
echo "dnf installs and upgrades from the repository, with gpgcheck and repo_gpgcheck on"
echo tampered >>/srv/repo/rpm/x86_64/repodata/repomd.xml
dnf clean all >/dev/null
if dnf makecache 2>&1 | grep -qiE "signature|verify|failed"; then echo "dnf refuses a tampered repomd"; else echo "dnf accepted a tampered repomd"; exit 1; fi
'

echo "== zypper (openSUSE Tumbleweed): install from the repository with signature checks on =="
docker run --rm -v "$MOUNT:/w:ro" opensuse/tumbleweed bash -euc '
rm -rf /srv/repo; mkdir -p /srv; cp -r /w/repo /srv/repo
rpm --import /srv/repo/agentmux.asc
zypper --non-interactive addrepo --gpgcheck --refresh file:///srv/repo/rpm/x86_64 agentmux >/dev/null
zypper --non-interactive --gpg-auto-import-keys refresh >/dev/null
zypper --non-interactive install -y agentmux >/dev/null; [ "$(cat /opt/agentmux/version)" = 1.1.0 ]
echo "zypper installs the signed rpm without --allow-unsigned-rpm"
'

echo "ALL PASSED"
