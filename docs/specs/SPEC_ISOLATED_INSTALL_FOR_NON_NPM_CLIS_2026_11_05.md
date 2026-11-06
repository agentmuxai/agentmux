# Spec: AgentMux installs its own isolated, pinned copy of all 19 agent CLIs

**Status:** active — approved by the operator 2026-11-05 ("ok, lets wrap that up, lets get all the harnesses isolated"), relayed by AgentY. PR 1 (goose: `cli_download.rs`, the Install button, lookup) is #6590 (merged); PR 2 (pinned uv, Aider, Kimi) is #6618 (merged); PR 3 (Grok) is #6627 (merged); PR 4 (Antigravity, Muse) is #6631 (merged); PR 5 (§9) is split in three, 5a (npm hardening) carrying this edit.
**Date:** 2026-11-05. Task #1571 (spec), #1572 (build). Author: Lazo. Verified against `agentmux` `main` @ `bf7e5b9863`.
**Builds on:** `SPEC_NINE_NEW_HARNESSES_2026_10_14.md` (the six providers), `SPEC_AGENT_CLI_ISOLATION_NOTE_2026_10_18.md` (what is isolated today), `SPEC_SIGNED_UPDATE_CHECKSUMS_2026_10_25.md` (same "pinned hash, no trust in the origin" idea).
**Research behind it:** a sourced fetch of each vendor's real distribution on 2026-11-05 (release assets, checksums, signatures, self-update switches, licences; downloads and Authenticode checks measured). Not committed.

## 1. Short answer

The operator saw the agent pane tell them to install Goose themselves and asked why AgentMux can't. It can. The installer is npm-only (`install_handlers.rs::installable_provider` refuses a provider with an empty `npm_package`), but none of the six that are left needs a vendor script:

- **Never run a vendor installer.** Fetch the pinned release file straight from the vendor's origin, check its SHA-256 (SHA-512 for Antigravity) against a value **compiled into AgentMux's provider registry**, unpack it into `<shared>/cli/<provider>/<pin>/` next to the npm installs, and publish it with the same lock and `.agentmux-install-complete` marker. The hash is in our binary, so the vendor's origin cannot swap the file (a hash that sits beside the file protects against a bad download, not a bad origin).
- **Aider is the one Python tool**: AgentMux fetches a pinned `uv` (checksum from its release), and `uv tool install --managed-python aider-chat==0.86.2` into AgentMux's folder.
- **The agent's launch finds AgentMux's copy first, for all 19.** The Install button the npm CLIs have replaces today's "install it yourself" hint.

## 2. The 19 in one table

"Pin" is the registry's `pinned_version`. "Verified by" is what AgentMux checks before it publishes the install. "Updates off" is set in the agent's environment (`auth_extra_env`), so the pin holds.

| Harness | Source | Pin | Verified by | Updates off | State home (`auth_config_dir_env_var`) | Container route |
|---|---|---|---|---|---|---|
| 13 on npm: claude, codex, gemini, copilot, cline, openclaw, dsh, qwen, pi, kilo, opencode, muxcode, crush | npm, `npm install --prefix` | per provider | npm's own integrity check (gap in §8) | already off | already set | npm into the per-agent volume (exists) |
| **goose** | GitHub release `aaif-goose/goose`, `goose-<target>.tar.gz` / Windows `.zip` (no Windows arm64) | 1.54.0 (1.57.0 is current) | SHA-256 from the release's asset digest, baked in; `gh attestation verify` as a build-time check of the baked value. Windows `goose.exe` is not code-signed | `goose update` is manual only: nothing to turn off | `GOOSE_PATH_ROOT` (set) | `ADD --checksum` / `sha256sum -c` of the `musl` tarball |
| **aider** | PyPI `aider-chat` through a pinned `uv` | 0.86.2 (is current) | PyPI wheel SHA-256 through a hashed requirements file (`uv pip install --require-hashes` into a venv `uv venv --managed-python` made), since `uv tool install` has no hash flag; `uv` itself by its release `.sha256` | `--no-check-update --analytics-disable` (set); add `AIDER_CHECK_UPDATE=false` | none exists (`~/.aider`, repo-local files): `HOME`-relative, noted not isolated | uv in the image, same hashed install |
| **kimi** | PyPI `kimi-cli` through the same `uv` | 1.52.0 (what the provider's flags were checked against; the project is archived, §9) | PyPI SHA-256, hashed install | none needed (a pure `py3-none-any` wheel, no updater found) | `KIMI_SHARE_DIR` (set) | same as aider |
| **grok** | `https://x.ai/cli/grok-<ver>-<platform>[.exe]` (+ on Windows `grove*.exe` and the MinGit zip, which has a `.sha256`) | 1.0.52 (still downloads; stable is 1.0.70) | **no upstream checksum exists**: SHA-256 taken once by us on a trusted machine and baked in (trust on first use, said so in the code), plus Authenticode subject `X.AI LLC` on Windows | `GROK_DISABLE_AUTOUPDATER=1` (set) | `GROK_HOME` (set) | `ADD --checksum` of the Linux build |
| **muse** | Meta's per-version manifest (`lookaside.facebook.com/.../muse/download/?channel=muse&version=<V>&file=manifest.json`), then the binary it names. Never Meta's launcher | 1.4.7-R6101.1 (manifest and files still download; stable is 1.4.14-R7309.1) | SHA-256 from the manifest, baked in; Authenticode `Meta Platforms, Inc.` on Windows | `MUSE_NO_AUTO_UPDATE=1` (set); running the binary without the launcher has no hourly swap | `XDG_CONFIG_HOME` (set) | `ADD --checksum` of the Linux build |
| **antigravity** | Google's per-platform manifest gives the URL and SHA-512 of the build at the time | the build that is current on the day of the PR (its server serves only the latest), URL + SHA-512 baked in | SHA-512 baked in; Authenticode `Google LLC` on Windows | `AGY_CLI_DISABLE_AUTO_UPDATE=true` (add) | none documented: state is `~/.gemini` (§7) | same, musl build |

## 3. Design

- **An install source per provider**, compiled in: `npm_package` (today) or an entry in `cli_download::SPECS` (PR 1; a `uv` kind joins in PR 2). PR 1 keeps the table beside the downloader instead of adding a field to all 19 `ProviderConfig` literals; a test holds each entry to its provider's pin, and a frontend parity test holds `managedInstall` to the table. An `Asset` is `{ os, arch, url, sha256, size, archive, exe_in_archive }`, all `&'static`, and gains an Authenticode subject when a PR needs one. A caller cannot name a URL or a hash: `installable_provider` stays the one gate and now means "has an npm package or a download". The unpacker takes only the one named executable out of the archive (no other entry is written, so no entry name can place a file).
- **`backend/cli_download.rs`** next to `cli_install.rs`: take the same cross-instance lock, download to a temp file in the install folder (size capped at the listed size + 1 MB, a stall timeout, progress lines through `publish_install_progress`), hash while streaming, refuse a mismatch and say which hash was expected, unpack (`.zip`/`.tar.gz`, no `..`, no absolute paths; the srv unpacker used for updates already refuses those), check the executable runs (`--version`), check the Windows Authenticode subject where one is listed, then write `.agentmux-install-complete`. `find_installed` learns the layout (`<dir>/bin/<cli>` for downloads, `<dir>/venv/bin/<cli>` for uv) so every resolver sees the same copy.
- **`uv`** is itself a pinned download (0.13.7; the hash baked in is GitHub's asset digest, which matches the `.sha256` sidecar) under `<shared>/tools/uv/<ver>/` (outside `cli/`, so the old-install pruner does not treat it as a leftover), with `UV_TOOL_DIR`, `UV_TOOL_BIN_DIR`, `UV_PYTHON_INSTALL_DIR`, `UV_CACHE_DIR`, `UV_NO_MODIFY_PATH=1` and `--managed-python` all inside the provider's folder. Without `--managed-python` uv silently reused the system Python in testing, so nothing would have been isolated. The 500 MB cache is cleared after a good install.
- **Lookup order** (`locate_provider_cli`): AgentMux's copy for every provider; a copy on the person's PATH is used only when they pick **Use the CLI on my PATH** for that provider (one setting, `cli:usepath`, a list of provider ids, default empty; a switch on the provider's row in Toolchain). Their own install is never touched. This replaces "PATH for the six". Built in PR 5b: with the setting on, their copy is looked for first and AgentMux's copy is the fallback if there is none, so switching it on never strands a launch; with it off, a copy on PATH is never used or reported as found.
- **Pin bumps** use the existing "update available" flow Toolchain shows for npm CLIs (`versionSource` in the catalog already names goose's GitHub repo); a new pin is a registry change with new hashes, so a bump is a PR a person reads.

## 4. What the person sees

- The agent pane's install hint (today `CliMissingOnPath`, AMX-CLI-004) becomes the **Install** button the npm CLIs have (AMX-CLI-001): progress lines, failure text with the expected and the actual hash, **Uninstall** (removes `<shared>/cli/<provider>/<pin>/`, nothing else). A provider that cannot be installed (legal hold, §7) keeps today's hint.
- Toolchain's card for each of the six shows the installed pin, its source and size, "update available", Install and Uninstall.
- The agent picker note that lists the six as exceptions goes away; the docs page and `catalog.ts` hints are updated in the PR that makes each one installable.

## 5. Windows, macOS, Linux

| | Windows | macOS | Linux |
|---|---|---|---|
| Targets | x64 (goose has no arm64; Grok, Muse, Antigravity, Kimi have arm64) | arm64, x64 | x64, arm64; musl where the vendor ships it (goose, Antigravity), else the one build (Grok, Muse), glibc floors unverified |
| Trust anchor | baked hash + Authenticode subject | baked hash; downloaded by srv, so no quarantine flag is set, to be confirmed on a Mac (Clare) | baked hash |
| Runs how | the exe directly, never a `.cmd` wrapper (the existing rule for prompt-in-argv and terminal-tier CLIs) | the binary | the binary |
| Looks checked | Area54 | Clare (Install button, a real install, an agent turn) | Maricon (same) |

## 6. Security

- No script from the internet is run. No `curl | sh`, no `irm | iex`, no Meta launcher, no `agy install`, no vendor PATH edit.
- One hash per file, in a reviewed PR, from a different trust path than the download. Grok has none upstream: that one is trust on first use, recorded in the code comment and the PR, and its Authenticode subject is the second check on Windows.
- Where a vendor does not sign (goose.exe) the build-provenance attestation is checked when the hash is captured, not at the person's machine.
- Grok's Windows installer also fetches `grove*.exe` and a MinGit zip (which has a `.sha256`). PR 3 does not: without them grove uses the git on PATH, which the installer itself treats as a note, not a failure. Add them later if a Grok feature turns out to need them.

## 7. Decisions and open points

1. **Kimi.** The Python `kimi-cli` is archived; the live product is Kimi Code CLI (native binary, manifest + `.sha256`, versions every 1–3 days). Its flags may differ from what `kimi.rs` passes (checked against 1.37), so moving the harness is its own task. This spec installs the pinned `kimi-cli` 1.52.0 through uv so Kimi works isolated now. Needs Python ≥ 3.12, which uv's managed Python supplies.
2. **Terms.** Muse Code has no published licence, and Google's Antigravity additional terms reportedly forbid third-party tools reaching the service (unclear for launching `agy` itself). The installs fetch the vendor's own file at the person's click, never bundle or re-host it. Checked 2026-11-06 (web-fetched summaries of the vendors' pages and forum threads, so quotes are near-verbatim): Antigravity's Additional Terms §6 calls "using third party software, tools, or services to access the Service (e.g. using OpenClaw with Antigravity OAuth)" a breach, and its FAQ reportedly names Claude Code and OpenCode too; but Google staff replies on three developer-forum threads (2026-10) say spawning the unmodified official `agy` locally, for one user and without extracting tokens, is permitted. Nothing was found on downloading or redistributing the binary. Muse Code has no licence or EULA, only Meta's general terms and an API acceptable-use policy; its docs invite driving `muse serve` from your own program. Neither says anything about fetching the binary at the person's click. Decision (Lazo, told to AgentY): proceed with both; this fetches the same file the vendor's own installer would, from the vendor's own server, so it adds no exposure beyond what the vendor-installer button already had. Written confirmation from Google or Meta is an open option. Grok's binary terms are unverified too. **Operator decision on Antigravity, 2026-11-06** (relayed by AgentY, the operator's words: "keep it"): #6631 ships in 0.60.18 as merged, with no hold for written confirmation from Google.
3. **Pins are old on purpose.** goose 1.54.0, grok 1.0.52 and muse 1.4.7 are what the harnesses were checked against; newer exist. Bumping them is separate, with its own hashes.
4. **Antigravity's pin can disappear** (latest-only server, no bucket listing). If its URL is gone the install fails with a clear message and the hint returns until the registry is updated.
5. **Antigravity state** has no documented override (`~/.gemini`). Isolating its login means `HOME`/`USERPROFILE` redirection, which affects what the agent sees; left as is (signs in once per machine) and noted in the docs.

## 8. The 13 npm CLIs: confirmed, with gaps

**Confirmed in the code:** `locate_provider_cli` returns AgentMux's install for an npm provider and returns `None` rather than PATH when it is missing (`cli_handlers.rs`, "never falls back to system PATH for npm-backed providers"); `install_pinned_cli` installs with `--prefix` into `<shared>/cli/<provider>/<pin>/`, no `-g`, no PATH edit; the auth folders are AgentMux's.

**Gaps (from the 2026-10-18 note, still open) and the fix in the last PR:**
- ~~The install uses the person's `~/.npmrc` (registry, proxy, scripts policy).~~ **Fixed in PR 5a:** every npm install passes `--registry=https://registry.npmjs.org/` and an empty file each for `--userconfig` and `--globalconfig` (npm refuses one file as both), made in the install folder. Their registry, scope routing and scripts policy no longer apply; a proxy in the environment (`HTTPS_PROXY`) still does, but a proxy set only in `.npmrc` no longer does, so an install behind such a proxy must move it to the environment. Checked with a real `npm install` on Windows.
- Node and npm come from the system. Said in the Install error; fetching a pinned Node is out of scope.
- `HOME` and `PATH` are the person's; this isolates the tools, not what an agent can do. Unchanged.

## 8a. Built in PR 2 (Aider, Kimi)

- `backend/cli_uv.rs`: the pinned uv, then per provider `uv venv --managed-python --python 3.12` and `uv pip install --require-hashes --only-binary :all: --no-deps -r <lock>` into `<install dir>/venv`, with `UV_CACHE_DIR`, `UV_PYTHON_INSTALL_DIR` and `UV_PYTHON_BIN_DIR` inside the install dir, `--no-config`, `--default-index https://pypi.org/simple`, and the person's `UV_*`/`PIP_*` index variables removed. The cache is deleted after a good install.
- The locks are `uv pip compile --universal --python-version 3.12 --generate-hashes` outputs in `backend/cli_locks/` (Aider 110 packages, Kimi 169, plus `pip`; the Kimi lock adds `setuptools` and `wheel`). A test holds every entry to an exact pin with sha256 hashes and no URL source.
- **Kimi's `ripgrepy` has no wheel** (sdist only), so it is built from its hash-checked sdist with `--no-binary ripgrepy --no-build-isolation`, setuptools and wheel coming from the same lock. uv builds an sdist before it installs anything, so they go in first, in a step of their own (macOS and Linux look checks caught the single-step install failing on a fresh venv); nothing is fetched unhashed.
- Measured on Windows x64 (Area54): Python download, venv and install take about 11 s for Aider (venv 514 MB) and 15 s for Kimi (219 MB); `aider --version` prints 0.86.2 and `kimi --version` prints 1.52.0. macOS, Linux and Windows arm64 are untested; arm64 Windows may lack wheels for some packages, which fails the install with uv's own message.
- Kimi's pin was empty and is now 1.52.0 (its registry entry, catalog and the install dir all key on it).

## 8b. Built in PR 3 (Grok)

- `cli_download::Archive::Raw`: the download is the program (no archive), so the checked file is copied to `bin/grok[.exe]` and nothing else is written. The six assets are `https://x.ai/cli/grok-1.0.52-<platform>[.exe]` (Windows x64 and arm64, macOS arm64 and x64, Linux x64 and arm64; one Linux build, no musl split).
- **The hashes are trust on first use**: xAI publishes no checksum, so each was taken once from x.ai on 2026-11-06 (the Windows x64 one equals a download made the day before) and baked in, with the size. A changed file fails the install and names the expected hash. Windows builds are Authenticode-signed (X.AI LLC); that is checked when the hash is captured, not on the person's machine.
- The provider already sets `GROK_DISABLE_AUTOUPDATER=1` and moves `GROK_HOME`, so the pinned copy does not update itself. The vendor installer's own folder (`~/.grok`) is no longer written by AgentMux.
- Grok's row is gone from the vendor-installer table, so the panel shows one Install, not two.

## 8c. Built in PR 4 (Antigravity, Muse)

- **Muse 1.4.7-R6101.1**: raw binaries from Meta's lookaside server, `https://lookaside.facebook.com/lookaside/muse/download/?channel=muse&version=1.4.7-R6101.1&file=muse-<platform>[.exe]`. The SHA-256 and size of each are copied from Meta's own manifest for that version, so this is not trust on first use. The binary is run directly (no launcher, so no launcher self-update; `MUSE_NO_AUTO_UPDATE=1` is set too). On Windows x64 the download matched the manifest, is Authenticode-valid (Meta Platforms, Inc.) and `--version` prints `Muse Code 1.4.7 (1.4.7-R6101.1)`.
- **Antigravity 1.5.2** (build `4951595792859136`, the build current on 2026-11-06; 1.5.1 was current the day before): Google's server lists only the latest, so the build's URL is pinned. Each file's SHA-512 matched Google's manifest when taken; the SHA-256 in the registry is of the same file. Windows is a raw exe; macOS and Linux are `tar.gz` holding one file, `antigravity`, installed as `agy`. Linux uses the static musl builds. The old 1.5.1 URL still downloaded a day after it was replaced, so a pin outlives its release for at least a day; if Google removes it, the install fails with the fetch error until the registry is bumped. The flags the harness passes (`--output-format stream-json`, `--dangerously-skip-permissions`, `--conversation`, `-p`) all exist in 1.5.2, and `agy --version` prints `1.5.2`. The provider now sets `AGY_CLI_DISABLE_AUTO_UPDATE=true` so the pin holds.
- The vendor-installer table (`VENDOR_INSTALLS`, the Run-the-vendor's-installer button) is now **empty**: every CLI is installed by AgentMux. The machinery stays, covered by a test-only fixture row, for a future CLI AgentMux cannot install itself; removing it is a decision for its owner.

## 9. Phases (one PR per install kind; merge on muxreview approval)

| PR | Content | Rough cost |
|---|---|---|
| 1 | `install` enum, `cli_download.rs`, lookup order, Install/Uninstall UI, **goose** (plain GitHub binary, all platforms), tests, this spec | 2–3 days |
| 2 | pinned `uv`, hashed venv install: **aider**, then **kimi** (1.52.0): built, §8a | 2 days |
| 3 | **grok** (TOFU hash, raw binary): built, §8b | 1 day |
| 4 | **antigravity** and **muse** (after the terms check): built, §8c | 1–2 days |
| 5a | npm hardening (§8): the public registry and an empty npm config for every install | built |
| 5b | the strict lookup order and the "Use the CLI on my PATH" setting (§3) | built |
| 5c | Toolchain Install and Uninstall (§4), docs and picker note, container route for all | 1–2 days |

**Tests:** hash mismatch refused and the partial removed; truncated and oversize download; a zip with `..` or an absolute path; two instances installing at once (the lock); a stale folder without the marker is cleared; the lookup order (own copy, then PATH only on request); `installable_provider` refuses an unknown id and a caller-sent URL; a registry test that every `Download` provider has an asset for each OS it claims and every hash is the right length. Live: a real install of each on Windows (Area54), macOS (Clare) and Linux (Maricon), then an agent turn.

Agent: lazo@Area54
