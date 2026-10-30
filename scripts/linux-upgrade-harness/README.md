# Linux package upgrade harness

Runs the `.deb`/`.rpm` upgrade path (`scripts/agentmux-install-update.sh`, the launcher's waiter in
`crates/launcher/src/upgrade_package.rs`) against real published packages, in containers, on Ubuntu 24.04,
Ubuntu 26.04 and Fedora 42. The harness also has Debian 13 and openSUSE Tumbleweed images (`debian13`, `tumbleweed`), which are untested. See
`docs/specs/SPEC_LINUX_PACKAGE_UPGRADE_AND_REPOSITORY_2026_10_24.md` §3.6.

```
FROM_VERSION=0.60.6 TO_VERSION=0.60.7 ./run.sh                       # every case, on ubuntu24 ubuntu26 fedora42
FROM_VERSION=0.60.6 TO_VERSION=0.60.7 ./run.sh -c <case> fedora42    # one case, one distro
./run.sh shell ubuntu24                                              # a root shell in a container with FROM installed
./run.sh clean                                                       # remove what it created (--all also drops the cache)
```

- Needs Docker, `curl` and `openssl`. Both versions' packages are downloaded from the GitHub release into `cache/`.
- `FROM_VERSION` must be 0.60.5 or later for the `real_*` and `waiter_*` cases (the helper and launcher that verify
  the signed manifest). `HELPER=<path>` tests another copy of the helper in place of the installed one.
- Each distro image is built once and kept (`amx-harness*`); a local HTTPS mirror (`mirror/mirror.py`) serves the
  releases' listings. `HARNESS_ID` gives a second user of the same machine its own network, mirror and images.
- What is real: the package manager, the packages, the signed manifest from github.com, the helper, the launcher
  binary's `--verify-update`, and the waiter script (read from `upgrade_package.rs`). What is a stand-in: `pkexec`
  (the container is root already, so it just runs the helper) and the command that starts AgentMux. The polkit
  prompt itself needs a desktop session and is checked by hand.

The helper's refusals of a bad file (wrong name, wrong package, not newer, another user's AgentMux still running,
going back twice, and so on) are in `scripts/agentmux-install-update.test.sh`, run against stub package tools.
They are not repeated here: the helper now asks the installed launcher to check every release against its
signed manifest, and the harness's mirror can't sign one.
