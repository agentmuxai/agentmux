# Spec: AgentMux Headless: one name for the server package, offered next to Standard on Windows, macOS and Linux (task #2598, build #2599)

**Status:** active. Approved by the operator, 2026-11-16 ("recommends are good", relayed by AgentY: D1 to D6 and the answers in section 8); AgentY's security conditions are in section 6. Author: AgentA@Area54. Follows `SPEC_HEADLESS_SERVER_PACKAGE_AND_LOCAL_WEB_UI_2026_11_12.md` (the package this renames) and Area54 #2583 (its first end-to-end runs).

The operator's words: "rename the server to name headless or background service. And integrate them into the 3 platforms. so windows has the standard and headless version, lunx and macos the same." Then about the downloads page: "the sections should be windows, macos, linux, with downloads and headless installs together per platform". Then: "we need the headless on windows too."

This is about what a person sees and downloads. Crates, the `agentmux-srv` program and every internal name stay as they are (section 3).

## 0. Decisions, with a recommendation each

| # | Decision | Recommendation |
|---|---|---|
| D1 | The name | **Headless** ("AgentMux Headless"). The pair is **Standard** and **Headless**. |
| D2 | The page | Three sections, Windows, macOS, Linux, each with Standard and Headless together; the "Server (no desktop)" section goes (section 1). It can ship ahead as a first small PR. |
| D3 | Release file names | `agentmux-headless-<version>-<os>-<arch>.tar.gz` (Windows `.zip`), published under the new names only (section 4). |
| D4 | The old install flags | `install.sh --server` and `--setup` keep working for two releases, silently, and are gone from `--help` and the page now. |
| D5 | Windows Headless | A real deliverable: a zip, installed by `install.ps1`, started at sign-in by a per-user scheduled task (section 5.1). Not code-signed, like the Windows installer today. |
| D7 | The release's other file names (section 9) | In the rename release: the two scripts become `agentmux-install.sh` and `agentmux-install.ps1` (also published under the old names for two releases) and the Headless files get the new pattern. The desktop files and the envelope, signature and manifest keep their names in that release and are renamed in a later one, after the updater has learned both names (9.3). |
| D6 | Updates | For the rename release, the update path stays "run the install command again", and `agentmux update` is built to say exactly that. A self-update is a later task. |

## 1. The page (first, because it is what the operator spelled out)

Today the download page has a block per desktop platform and a separate "Server" section (the page's own server section); the front page's "Another platform, a package manager, the server, or a checksum" line points at it.

New layout, on the front page's block and on the All Downloads page:

```
Windows                     macOS                       Linux
 Standard                    Standard                    Standard
   installer, portable,        dmg (Apple Silicon)         AppImage, deb, rpm
   and the others the          Intel Macs: none            (x64)
   page lists today
 Headless                    Headless                    Headless
   Not available yet           Apple Silicon:              x64 and arm64:
   (Windows is next)           one command + the file      one command + the files
```

- Each platform shows Standard first, then Headless under it; every file keeps its size and SHA-256 line.
- One line says what each is. Standard: "The desktop app: windows, panes and agents on your screen." Headless: "AgentMux with no desktop window. It runs in the background and you open it in a browser on the same computer; nothing listens on the network."
- Windows says plainly "Headless is not available on Windows yet" until the Windows package ships (section 5.1); the line becomes the command then. Intel Macs and Linux arm64 keep their existing sentences: neither has a desktop build, and Headless has no Intel Mac package either; arm64 Linux has Headless only.
- The commands shown are the ones of the released `agentmux --help` (the landing already checks this on a CI runner); they do not change in this PR.

**Can it ship ahead of the rest? Yes, as a first small PR for Winny (landing).** It needs no change in the agentmux repo: the page's data comes from the release as it is, the three platforms' files are the ones the release has today, and the commands stay `install.sh --server` and `agentmux ...`. The prose and headings say Headless; the one visible mismatch is that the command still says `--server` until the rename release (section 4). Accept that for one release, or hold the wording until the rename ships if the operator would rather have one coherent step.

## 2. The name

- **Headless**, because it is true of every install (there is no window), while "background service" is only true after `agentmux setup` has installed the login service, and a person who never runs setup would be told something false. The word is already in the product: the data channel is `headless` and the server's flag is `--headless`.
- "Standard" for the desktop app is a page word only; the app's own name stays "AgentMux".
- Writing rule for user text: "AgentMux Headless" the first time, "Headless" after; never "the server package", "server-only" or "no-desktop" in anything a person reads.

## 3. Every place the name shows

Changes (agentmux repo unless stated):

| Where | Now | New |
|---|---|---|
| Release files (`scripts/package-server.sh`) | `agentmux-server-<v>-<os>-<arch>.tar.gz` | `agentmux-headless-<v>-<os>-<arch>.tar.gz`; Windows `.zip` |
| The package's top folder | `agentmux-server-<v>-<os>-<arch>/` | `agentmux-headless-<v>-<os>-<arch>/` |
| Signed manifest | lists the file names above | lists the new names (the manifest is keyed by file name; no key is renamed) |
| `install.sh` | `--server`, `--setup`, "installing the AgentMux server", `~/.agentmux/server`, the `agentmux-server` link | `--headless`, `--setup`, "installing AgentMux Headless", `~/.agentmux/headless`, the `agentmux-headless` link; an old `~/.agentmux/server` is left in place and named, with the command to remove it (its server is stopped first), and `agentmux-headless setup --service` points its login service at the new install |
| `install.ps1` | none for the package | `-Headless` (Windows) |
| `agentmux` CLI | `about = "Run and manage the AgentMux server on this machine"`; messages "the server is not running", "the server package is for Linux and macOS in this release", "cannot start the server" | "AgentMux Headless"; messages say "AgentMux Headless" or "it" once the sentence has named it; `invoked_name()` knows `agentmux-headless` |
| The sign-in page and the browser's not-signed-in card | "headless server" wording in the card's text | "AgentMux Headless" |
| Release notes lines (`release-core.yml`: "the server package is built for ...") | server package | Headless |
| Workflow and job names | `build-server-package.yml`, "Server package (os-arch)", "Build — server package" | `build-headless-package.yml`, "Headless package (os-arch)", "Build — Headless package". Check which job names branch protection requires before renaming; if one is required, rename it in the same change that updates the rule (AgentY). |
| The landing's download model, its All Downloads page and Get AgentMux block, its data file and its checks of the page against the release | server | Headless; the model's slot ids `server.linux-x64` become `headless.linux-x64` |
| docs.agentmux.ai (agentmux-docs) | a page named "Headless server" (added by the docs pass, agentmux-docs #297) | renamed "AgentMux Headless", with the new commands once the rename release is out (P5) |
| README | the server package paragraph | Headless |

Stays (internal, or a name a person never reads):

- The crates (`agentmux-srv`, `agentmux-srv-core`, `agentmux-common`), the `agentmux-srv` program, `AGENTMUX_*` variables, the `headless` channel and `~/.agentmux/channels/headless`, the `--headless` flag.
- The login service's own names (the launchd label `ai.agentmux.server...`, the systemd unit `agentmux...service`): a person reads them only in the service manager's own list, and renaming one would orphan a service that is already installed.
- `agentmux-server.json`, the settings file `setup` writes inside the channel folder: renaming it is a migration for no gain a person sees.
- The repository variable `SERVER_PACKAGE_MACOS_SIGNING` (the switch for Developer ID signing of the macOS package stays off until the operator has OK'd it; renaming a variable is not the place to touch it).
- Old specs and old changelog lines: the past stays in code docs (the alpha rule).

## 4. The rename release

Order, so nothing fails in between:

1. **The landing learns both names first.** The landing matches a release file by shape in tiers; the new shape goes first and the old one second, so the landing builds against the release before the rename and the one with it. The v0.60.28 fixture stays valid.
2. **The agentmux repo renames** (installers, packaging, CLI text, workflow) and a release is cut.
3. **The landing drops the old tier** one release later.

Questions the task asked:

- *Does an installed server find its upgrade when the asset's name changes?* Nothing in an installed server fetches an asset by name: it has no `update` command today (the only update path is running the install command again, which downloads the current release). A grep of the crates for the old file name finds only the CLI's own program-name check and the settings file name. So no installed server breaks.
- *An old copy of `install.sh` against a new release* asks for the old name and gets the clear existing message ("this release has no server package for this system"). The site and the release always carry the matching script, so this needs a saved copy of the script or an old tag pinned by hand. Publishing both names for one release would double the upload for that case; I recommend not doing it.
- *Alpha rule:* user docs forget the old name. `--server` and `--setup` keep working silently for two releases (a one-line alias in the script), then go. `agentmux-server` (the PATH link) is replaced by `agentmux-headless` on the next install.
- *`agentmux update` and `uninstall`* (#2583 item 5): `update` prints the install command for this platform; `uninstall` stops the server, removes the service and the install folder, and leaves the data folders (the person's agents and history) unless `--remove-data` is given and confirmed.

## 5. Per platform

All three keep the one-writer rule for the shared folder (#7432): Headless refuses to start beside a desktop of another version and says so, and the page says Standard and Headless of the same version run side by side.

| | Standard | Headless |
|---|---|---|
| Windows | installer, portable, MSIX (the installer is not code-signed yet) | new zip (5.1), x64 |
| macOS | dmg (Apple Silicon only) | Apple Silicon only, tarball; Developer ID signing off until OK'd |
| Linux | AppImage, deb, rpm (x64) | x64 and arm64 tarballs; the .deb and .rpm of Standard are unrelated |

### 5.1 Windows Headless (new)

- **Package:** `agentmux-headless-<v>-windows-x64.zip`: `bin\agentmux.exe` (the CLI), `bin\agentmux-srv.exe`, `bin\agentmux-bashwrap.exe`, the built web UI in `web\`, the same layout as the tarballs. It is built on a Windows runner as one more leg of the package workflow (the release already waits for Windows runners for the desktop).
- **Install:** `irm https://agentmux.ai/install.ps1 | iex` with `-Headless`: downloads the zip, checks it against the signed manifest (the same check the Linux script makes), extracts to `%LOCALAPPDATA%\AgentMux\Headless`, adds `bin` to the user's PATH. The link rule is the Linux one: `agentmux-headless` always, `agentmux` when nothing else has the name.
- **Runs with no window:** the programs are console-subsystem, which makes a console window appear if a GUI session starts them. `agentmux start` therefore starts the server detached with no window (the spawn is already flagged `no_window` in the CLI's helpers), and the sign-in task (below) starts it through a small GUI-subsystem stub that never allocates a console. **This is the one part to prove on Windows 10 and 11 before the build is sized**; if a stub is not enough, the fallback is a hidden-window shortcut in the user's Startup folder.
- **Starts at sign-in:** `agentmux setup` registers a per-user Scheduled Task with the logon trigger, "run only when the user is logged on", least privilege, no password stored. It is created the way the desktop's own autostart is (an XML task definition; the launcher already does this). It is not a Windows Service: a service starts before sign-in, as another account, without the person's profile.
- **Stop and status:** no signals on Windows. `stop` asks the running server to shut down over its loopback address with the session credentials the CLI already holds, and `status` identifies the process by the program's image (the `image_of` stub in `process.rs` is empty on Windows today and is the main port).
- **Update:** run `install.ps1 -Headless` again (D6).
- **Signing:** Authenticode, like the Windows installer, is not in place. A zip downloaded by a browser carries the browser's mark and the executables inside may meet SmartScreen; the install script's download does not carry the mark, but Defender may still judge an unsigned program. The package is unsigned until the desktop's signing is decided; the page says nothing about "verified publisher". To be tried on a clean machine.

### 5.2 macOS and Linux

Unchanged in what they are: `install.sh --headless [--setup]`, a user service (launchd or systemd --user) that starts at login after `agentmux setup`, updates by running the command again. Linux arm64 has Headless only (no desktop build). An Intel Mac has neither a Standard nor a Headless package, and no plan for one; the page says so in one line.

## 6. Security requirements (settled with AgentY before the build; the analysis behind them is kept privately)

These are rules the build keeps and tests, not a design discussion.

1. **What listens where does not change.** Loopback only, one fixed port by default, nothing on the network. The session pair (cookie plus token) and the Host and Origin checks stay as they are on every platform. The rename touches names and packaging, not listeners.
2. **Windows files are the installing user's alone.** The install folder, the data folders under `.agentmux\channels\headless` and the key file are readable and writable by the installing user only. `setup` and `doctor` check this and `setup` refuses, in words that name the folder and what to change, when it does not hold. A test on a Windows runner covers both a folder that fails the check and one made by `setup`.
3. **The Windows listener is bound for its own exclusive use** of its address, and a test binds the port twice and expects the second to fail.
4. **`agentmux open` and `logout` keep their server proof** (the server proves it holds the key file and is bound to the address, before anything is sent; `crates/srv/src/server/local_proof.rs`) on every platform. If the build finds that this does not hold on one platform, it stops and goes to AgentY first.
5. **Start at sign-in:** a per-user Scheduled Task, as the signed-in user, least privilege, no stored password, no service, no SYSTEM, no admin prompt; `setup` never needs elevation. The task's action points only at a path inside the user's own profile (`%LOCALAPPDATA%\AgentMux\Headless\bin\...`); `setup` refuses to write a task whose action is anywhere else, and `doctor` warns when the action is not the installed program. The no-console stub is built from this repository in the release, shipped inside the zip and listed in the signed manifest; it is never fetched by `setup` or at sign-in.
6. **The streamed browser route** (task #18045, headless only, opt-in by an environment variable, 404 otherwise) is **not offered by Windows Headless**: it is not compiled in or enabled for that package. The rename does not change the flag or its gate on Linux and macOS.
7. **Downloads:** every file is checked against the signed manifest. The manifest is keyed by file name, so the rename needs the new names in the manifest before the landing or a script can find them (section 4).

## 7. Build order, in small PRs, and who

| PR | What | Seat |
|---|---|---|
| P0 | The page: Windows, macOS, Linux, Standard and Headless together; "not available yet" on Windows; tests of the download model | Winny (landing). No agentmux-repo change. |
| P1 | The landing accepts both file-name shapes (new first), for the Headless files and for the install scripts (`agentmux-install.sh` and `.ps1` as well as the old names) | Winny |
| P2 | The rename in the agentmux repo: packaging, `install.sh`, CLI text, release notes, workflow names, `agentmux update` and `uninstall`; one release | AgentA (install.sh edits go through Winny's review, who owns it) |
| P3 | The CLI on Windows: process image, stop, status, the no-window start, the logon task | AgentA, with Posa or Lazo for the Windows runs |
| P4 | The Windows package leg in the package workflow, and the zip | Lazo or Posa (release workflow wiring; the diff goes to AgentY before merge) |
| P5 | `install.ps1 -Headless`, the page's Windows command, the docs page, the README, the landing's old tier dropped | Winny and Agent2 |

P0 and P1 can start the day the spec is approved. P2 waits for P1 to be live. P3 and P4 can run in parallel with P2; the Windows line on the page changes only when P5 lands. Each is its own PR with its own changeset; none renames a crate.

## 8. Answers to the open questions (approved 2026-11-16)

1. The old flags (`--server`, `--setup`) are kept silently for two releases (D4).
2. "Standard" stays the page word for the desktop app.
3. Windows on ARM is not planned: Windows Headless is x64 only.

Build order and seats are section 7; P0 (the page) is started by Winny (its own task on the landing's board), and P1 follows it. The Windows 10 and 11 no-console start (section 5.1) is tried in the builder's own build, off the shared screen. Section 6's requirements are checked with AgentY before P3 is built.

## 9. The release's file names (the naming pass; the operator, 2026-11-16: "should the assets have more specific names? ... a random Install.ps1 there")

### 9.1 Every file of a release today, and the pattern

Release v0.60.29 lists: `AgentMux-<v>-x64-setup.exe`, `agentmux-<v>-x64-portable.7z`, `AgentMux_<v>_x64.msix`, `AgentMux_<v>_arm64.dmg`, `AgentMux_<v>_amd64.AppImage`, `AgentMux_<v>_amd64-portable.tar.xz`, `AgentMux_<v>_amd64.deb`, `AgentMux-<v>-1.x86_64.rpm`, `agentmux-server-<v>-<os>-<arch>.tar.gz` (three), `AgentMux-<v>-manifest.json`, `AgentMux-<v>-envelope.json`, `AgentMux-<v>-envelope.json.sig`, `install.sh`, `install.ps1`. Three spellings of the name (`AgentMux-`, `AgentMux_`, `agentmux-`) and three of the architecture (`x64`, `amd64`, `x86_64`).

The pattern for the files we name ourselves: `agentmux-<version>-<os>-<arch>[-<kind>].<ext>`, lower case, hyphens, one word per architecture (`x64`, `arm64`).

| Today | New | Note |
|---|---|---|
| `AgentMux-<v>-x64-setup.exe` | `agentmux-<v>-windows-x64-setup.exe` | |
| `agentmux-<v>-x64-portable.7z` | `agentmux-<v>-windows-x64-portable.7z` | |
| `AgentMux_<v>_x64.msix` | `agentmux-<v>-windows-x64.msix` | the Store package's identity is inside the file, not its name |
| `AgentMux_<v>_arm64.dmg` | `agentmux-<v>-macos-arm64.dmg` | |
| `AgentMux_<v>_amd64.AppImage` | `agentmux-<v>-linux-x64.AppImage` | |
| `AgentMux_<v>_amd64-portable.tar.xz` | `agentmux-<v>-linux-x64-portable.tar.xz` | |
| `AgentMux_<v>_amd64.deb` | `agentmux_<v>_amd64.deb` | the Debian shape (`name_version_arch`, `amd64`) stays; only the name part is lower case, like the package |
| `AgentMux-<v>-1.x86_64.rpm` | `agentmux-<v>-1.x86_64.rpm` | the RPM shape (`name-version-release.arch`) stays; lower case |
| `agentmux-server-<v>-<os>-<arch>.tar.gz` | `agentmux-headless-<v>-<os>-<arch>.tar.gz`; Windows `.zip` | section 3 (the Headless packages keep their own order: the product word, then the version) |
| `AgentMux-<v>-manifest.json`, `-envelope.json`, `-envelope.json.sig` | `agentmux-<v>-manifest.json`, `-envelope.json`, `-envelope.json.sig` | see 9.2: these are read by name in every installed app |
| `install.sh`, `install.ps1` | `agentmux-install.sh`, `agentmux-install.ps1` | the site keeps serving them at `agentmux.ai/install.sh` and `/install.ps1`; every typed command stays the same |

### 9.2 Who finds its file by name today (read from the code)

| Consumer | How it finds its file | On a release with only the new names |
|---|---|---|
| In-app upgrade, every install kind (srv, `update/check.rs` and `install.rs`) | builds the exact name for its kind and looks it up in the GitHub release's file list (`asset_name`: `agentmux-<v>-x64-portable.7z`, `AgentMux-<v>-x64-setup.exe`, `AgentMux_<v>_arm64.dmg`, `AgentMux_<v>_amd64.AppImage`, `AgentMux_<v>_amd64-portable.tar.xz`, `AgentMux_<v>_amd64.deb`, `AgentMux-<v>-1.x86_64.rpm`) | **finds nothing: the update is not offered.** |
| The launcher's restart step (Windows `AgentMux-<v>-x64-setup.exe`, macOS `AgentMux_<v>_arm64.dmg`, the Linux AppImage and `.deb`/`.rpm` names) | checks that the staged file has exactly that name before it runs it | refuses the file |
| Release verification (`update_manifest::file_names`, used by srv and the launcher's `--verify-update`) | `AgentMux-<v>-envelope.json`, `.sig` and `-manifest.json` | **no verified update at all.** |
| The `.deb`/`.rpm` root helper (`agentmux-install-update`, shipped inside the installed package) | `AgentMux_<v>_amd64.deb` or `AgentMux-<v>-1.x86_64.rpm`, and `AgentMux-<v>-envelope.json` | fails for every installed package |
| apt and rpm repositories (`publish-linux-repo.yml`) | the two package names, from the release | the publisher is changed with the rename (Lazo or AgentY) |
| winget manifests (Winny) | the installer URL names the setup file; a new version's manifest is made from the new name | follows the rename; no installed version is affected |
| The landing (Winny) | exact name shapes per slot; the install scripts by `install.sh` and `install.ps1` | accepts both shapes first (P1) |
| `install.sh`, `install.ps1` | the manifest's names, from the release; `AgentMux-<v>-manifest.json` | updated with the rename; an old copy of the script against a new release is the case in section 4 |
| The second signature (#7447, Winny) | names the envelope | follows the envelope's name |
| Docs and README | typed commands and links | `releases/latest/download/install.sh` is a link people have pasted: keep it working (9.3) |

So an installed AgentMux finds its upgrade only by these exact old names, in an app that is already installed and cannot be changed.

### 9.3 What ships, and when

The operator's condition: **no installed AgentMux loses its update.** Publishing every desktop file under both names would hold it, but a release would carry about 1.4 GB twice and the file list the operator found odd would be twice as long, so it does not make the list tidier. So:

1. **In the rename release (P2):** the two scripts get their new names and also stay under the old names for two releases (a few tens of kilobytes; `releases/latest/download/install.sh` is a link people have pasted, and the landing, docs and Winny's checks name it); the Headless files take the new pattern (no installed app looks for them). Nothing else is renamed. This part can ship now without touching any update.
2. **A later release R2 (its own spec task, proof first):** the updater (srv, the launcher, `file_names`, the `.deb`/`.rpm` helper and the verifier) learns both names: it looks for the new name first and falls back to the old one, and `update_manifest::file_names` accepts the new envelope names. R2 still publishes the old names only. Proof: an installed old build and an installed R2 build each find and take their update on every platform (Windows setup and portable, macOS, AppImage, Linux portable, deb and rpm), from a release that carries both shapes.
3. **R3, once R2 is the oldest version that has to upgrade in place** (the `UPGRADE_FLOOR` raised to R2: AgentY's policy): the release publishes the new names only. A version older than the floor does not upgrade in place and says so, as the floor already does.

Per file: scripts and Headless files: rename now. Setup, portable, dmg, AppImage, portable tar, deb, rpm, envelope, signature, manifest: wait for R3. MSIX: the Store updates by its own identity and nothing reads it by name; it follows in R3 only to keep one change.

If a consumer I did not find reads a file by name, that file stays under its old name until the consumer is changed.

### 9.4 Seats

- Updater and launcher name handling (R2), the in-app upgrade on macOS and Linux, and the Store: QTest. The Windows in-app upgrade (setup.exe and portable): Lazo. The `.deb`/`.rpm` helper and the apt and rpm publisher: Lazo or AgentY.
- winget manifests, the landing's picker, `install.sh` and `install.ps1` edits, the second signature, docs and README: Winny (the install scripts also Agent2).
- The floor (R3): AgentY. The release workflows' file-name changes: their diff goes to AgentY before merge.
- Me: the spec, P2 (the rename release: scripts, Headless files, CLI) and R2's spec task.
