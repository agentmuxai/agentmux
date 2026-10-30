# Spec: a developer image for container agents (#11091; build #11092)

**Date:** 2026-10-29 (measured 2026-10-30) · **Owner:** Maricon-2 · **Status:** approved 2026-10-30 ("go with your recommendations": decisions (a) to (d) in §9 as recommended); built in small PRs
**Related:** `SPEC_CONTAINER_AGENTS_WORK_FOR_EVERYONE_2026_10_07.md` (the base image), Loap-3's CPU/memory limits spec (#11089, `SPEC_CONTAINER_AGENT_RESOURCE_LIMITS_2026_10_29.md`), #5843 (an idle container moves to the current image).

## 1. The problem

`docker/Dockerfile.agent-agentmux` is `node:24-slim` with git, gh, python and the two AgentMux binaries. Rust exists only in its builder stage, and there is no C compiler, so a container agent cannot build a Rust project, and a user hits that before we do. We want container agents to do AgentMux's own work (check, test, `task dev`, screenshot) with nothing but Docker on the host.

## 2. The image

A second image, **developer**, built `FROM` the exact agent-base image of the same release, so it carries the same `agentmux-mcp` and `agentmux-bashwrap`.

| Layer | Contents | Size (uncompressed) |
|---|---|---|
| agent-base | as today | 484 MB |
| Linux build packages | `build-essential`, `pkg-config`, `cmake`, `ninja-build`, and every package CI's Linux jobs install (`ci-pr.yml`: libwayland, libxkbcommon, libdbus-1, libxcb1; `build-linux.yml`: libgtk-3, glib, pango, cairo, gdk-pixbuf, atk) | 787 MB |
| Headless display + CEF runtime | `xvfb xauth xdotool scrot fonts-liberation`, libnss3, libgbm1, libatk-bridge, libcups2, libdrm2, libx* and the rest CEF loads | 118 MB |
| Rust | rustup, minimal profile, pinned version, clippy, rustfmt | 652 MB |
| Tools | go-task, cargo-nextest (the projects' own prebuilt binaries, per architecture) | 89 MB |
| **Total** | | **2.67 GB** |

- **Size on the wire: about 660 MB** for the whole image (`docker save | gzip -6`, the registry's layer compression), of which about 120 MB is the base layers a user of the standard image already has. A user with the base pulled downloads about 540 MB more. Pull time is that divided by their line (about a minute at 100 Mbit/s); I could not measure a real pull before the image is published, so this is an estimate to confirm on the first publish.
- **Rust is not pinned in the repo today.** There is no `rust-toolchain` file: every CI job installs floating `stable` (`dtolnay/rust-toolchain@stable`). The brief says "pinned to the repo's rust-toolchain"; there is nothing to read. **Decision for the operator (needs yes):** add `rust-toolchain.toml` (`channel = "1.98.1"`, the version charlie and the prototype use) as the one source for CI, the image and local builds, bumped by a normal PR when we move. The image's Dockerfile reads the channel from that file. The cost: CI stops floating on `stable`, so a new Rust release reaches us on our schedule, not GitHub's. If the operator would rather not pin CI, the fallback is an `ARG RUST_VERSION` in the Dockerfile, bumped by hand, and the image can lag behind CI.
- **Generic for any Rust project:** the variant is a general Rust toolchain plus a C toolchain; AgentMux's own needs (GTK/Wayland headers, Xvfb, CEF libraries) are extra weight a plain Rust project does not use. One variant is simpler and the superset works for both, so v1 ships one. If size matters, the cheap split is a lean `-rust` (about 0.9 GB: base + C toolchain + Rust + tools) next to the full `-developer`; the layers above say what each costs.
- **Image labels** (read by AgentMux when it creates the container, with no extra pull): `ai.agentmux.recommended-min-memory-mb` (see §5) and `ai.agentmux.cache-dirs` (see §4). Any custom image can set them too.
- **Environment:** `CARGO_HOME=/home/agent/.cargo`, toolchain in `/usr/local/rustup` (read-only, from the image), `agent` user as in the base. **The image must not set `CARGO_TARGET_DIR`** (the prototype did, and it broke `task dev`: see §6).

## 3. Publishing

`container-image.yml` gets a second job, `build-and-push-developer`, `needs: build-and-push`:

- It builds `docker/Dockerfile.agent-developer` with `--build-arg BASE=ghcr.io/agentmuxai/agent-base@<digest of the job before>`, for `linux/amd64,linux/arm64`. Building from the digest, not the tag, means a developer image can never sit on a different base than its release's signed one. The arm64 half needs only apt and rustup downloads under QEMU (nothing compiles), but I have **not** built arm64; its build time is unmeasured. The local amd64 build took 2 m 47 s.
- Tags go in the **same package** as the base, `ghcr.io/agentmuxai/agent-base`: `<version>-developer`, `<major.minor>-developer`, `developer`; a dispatch run tags `dispatch-<sha7>-developer`. A new package would start private, and only an organization owner can make it public in the GitHub UI (the file's own header says so); a tag in the existing public package needs no new step. The same anonymous-pull check runs against the first developer tag.
- The job's digest is a workflow output; `release-core.yml` adds it to the signed manifest's `images` list as `agent-base-developer`. The manifest's `images` is already a list (`ManifestImage { name, ref, digest }`), so older apps ignore the entry.
- `agent_image_digest.rs` looks the entry up by variant (today it hard-codes `agent-base`), so a developer container is created from the signed digest, tagged locally as `agent-base:<version>-developer`, exactly as the standard one is.

## 4. Choosing it per agent, and #5843

**Setting.** Nothing new is stored: the agent's existing `agent:container_image` carries it.

- `ghcr.io/agentmuxai/agent-base:developer` means "this AgentMux version's developer image", the way an empty value or `:latest` means this version's standard image. `resolve_container_image` maps it to `agent-base:<version>-developer`. Add `versioned_developer_image()` beside `versioned_agent_image()`.
- The new-agent form's container section gets an **Image** choice: *Standard* (default), *Developer* (names the size: "about 660 MB, for building software"), and *Custom* (the text field that exists today). The anonymous registry check the form already runs (`ContainerImageCheckCommand`) runs on the chosen image.
- A user can type `agent-base:0.60.9-developer` to pin a version, as with the standard image today.

**#5843 (an idle container on an older image is updated).** `image_blocks_update` decides it by comparing the resolved image to `versioned_agent_image()`. With the change it compares to the *variant's* versioned image:

- developer (floating `:developer`) → follows the release, like the default: an idle container on an older developer image is removed and recreated on the current one;
- a version-pinned developer tag or a digest → "pinned", as today;
- any other image (`rust:1`, the user's own) → "runs its own image", as today.

**What the update keeps.** The update removes the container and keeps volumes. That makes the next section matter: tools installed inside the container (including a user's `cargo install`) are lost on an update, so **cargo's registry and the build output must live in volumes**, or every release throws a 3 GB cold build away.

**Mounts.** `agent_home_mounts` reads the image's `ai.agentmux.cache-dirs` label (`/home/agent/.cargo`) and adds a per-agent named volume for each, `agentmux-cache-<container>-<n>`, next to the existing `agentmux-claude-<container>` and `agentmux-cli-<container>`. The volume is initialised from the image's directory, so `agent` owns it from the first start (checked in the prototype). Containers made before the change have no such mount; the existing drift check recreates them on the next start like any other mount change.

**Build output (`target/`).** Today it stays inside the workspace folder, which is a host bind mount: fine on a Linux host (that is what every number below measures), slow and case-insensitive on Windows and macOS, and it sits in the user's folder (3 GB for a check, 12 GB after `task dev`). The right home is a volume, but the Taskfile and 9 scripts hard-code `target/` (37 lines in `Taskfile.yml`, and `resolve-cef-runtime.sh` does `find "$REPO_ROOT/target"`), so `CARGO_TARGET_DIR` breaks them. **Build step 0 (done in the first PR):** the Taskfile gets a `TARGET_DIR` variable (`CARGO_TARGET_DIR` if set, else `target`) used by the Linux and macOS dev build, the dev session staging and the clean tasks, and the two CEF resolver scripts read `CARGO_TARGET_DIR` too. Windows and the packagers (`package-*.sh`, `stage-linux-runtime.sh`) keep reading `target/`: nobody sets the variable there, and `check-bundled-tools.sh` greps their literal paths. Once that is in, the image's cache-dirs label gains `/home/agent/target` and the env sets `CARGO_TARGET_DIR`. Until then the variant leaves it unset and builds in the workspace.

`node_modules` (600 MB, `npm ci` 19 s) stays in the workspace for now. On Windows and macOS hosts it is the slow bind mount again; a volume is the follow-up if someone measures it hurting there.

## 5. Sizing and cleaning

**Sizes, measured** (one clone of agentmux at `be2d8f8fe`, Linux):

| What | Size |
|---|---|
| cargo registry and git (`~/.cargo`) after `cargo check --tests -p agentmux-srv` | 475 MB (619 MB after `task dev`) |
| `target/` after that check | 3.2 GB |
| `target/` after `task dev` (release profile with the dev overrides) | 12 GB |
| `dist/` (staged CEF runtime and binaries) | 3.1 GB |
| `node_modules` | 600 MB |

A developer agent that has built AgentMux holds about 16 GB beyond the image, mostly in `target/` and `dist/`. Docker's local volume driver has no size limit.

**Cleaning, today:** nothing removes an agent's volumes when the agent is deleted. The only callers of `remove_volume` are tests, so `agentmux-claude-*` and `agentmux-cli-*` volumes outlive their agent already; at a few hundred MB that went unnoticed, at several GB it won't. The build must:

- label every volume it creates (`ai.agentmux.agent=<container>`, `ai.agentmux.kind=cache`), so a sweep can find them;
- remove an agent's volumes, new and old, when the agent is deleted (and tell the user how much space that frees);
- add **Clear build cache** to the agent's container menu (removes the cache volumes and, with the Taskfile change, `target/`), and show the sizes there (`docker system df -v` reports volume sizes without scanning);
- warn once in the agent's pane when its caches pass 20 GiB.

Cargo leaves artifacts of an older compiler in `target/` after a Rust bump, so a developer agent's `target/` grows until cleared. The same menu entry is the answer; automatic pruning (`cargo-sweep`) can follow if it proves needed.

**With Loap-3's limits spec (#11089, build #11090; his constant is `RECOMMENDED_MIN_MEMORY_LABEL` in `container/limits.rs`):** the label name `ai.agentmux.recommended-min-memory-mb` is agreed. **The value must be 16384, not the 8192 the prototype carried.** Measured peaks (no limit set, 16 CPUs, 52 GB host): `cargo check --tests -p agentmux-srv` cold 9.8 GiB; the `task dev` build 14.6 GiB. A Docker Desktop VM on a laptop is often smaller than that, so `CARGO_BUILD_JOBS` and `MAKEFLAGS=-j` set to the CPU limit (his §9.5) is what keeps a smaller limit from being killed; the image sets neither, so there is no clash. The image gives no CPU count of its own.

## 6. Proof

Charlie (Linux, 16 CPUs, 52 GB), commit `be2d8f8fe`, one container on the prototype image against native cargo and Node, each with a fresh `CARGO_HOME` and target so the cold numbers compare. amd64 only.

| Step | Native | Developer container |
|---|---|---|
| `git clone` | n/a | 8 s |
| `cargo check --tests -p agentmux-srv`, cold | 272 s | 241 s |
| Same, warm (touch `crates/srv/src/main.rs`) | 83 s | 54 s |
| `npm ci` | 12 s | 19 s |
| Frontend typecheck (`tsc -p tsconfig.citypecheck.json`) | 27 s | 30 s |
| `task dev` to a rendered window, cold (compile time only) | 12.4 min (30.6 min wall: 18.2 min of it queued for charlie's shared build slot) | about 14 min: backend 7.6 min, host, CEF and launcher about 5 min, window up about 1 min, plus 19 s for npm |

The container is not slower than native in any compile step (cold `cargo check` 11% faster, warm 35% faster; `npm ci` and the typecheck a few seconds slower). Charlie is shared with other agents' builds and the runs were not back to back, so I read this as "no container penalty on Linux", not as the container being faster. `task dev` ran in the container under Xvfb, and a screenshot (`scrot`) of the window shows the app rendered (the Agent pane with its templates, the CPU and Swarm panes, the status bar showing v0.60.7 DEV). The proof exposed four things the image and Taskfile must handle, each in the build:

1. `CARGO_TARGET_DIR` breaks the Taskfile and `resolve-cef-runtime.sh` (§4, build step 0). A symlink `target → volume` is not a fix: the resolver's `find` does not follow it.
2. CEF's sandbox does not start in a container (the host's user-namespace policy blocks it). The app says so and names `AGENTMUX_UNSAFE_NOSANDBOX=1`; the container is the sandbox, so the developer image sets it for `task dev` only (documented in the image, not set globally).
3. The dev build uses the **upstream** CEF from cargo's cache, not AgentMux's patched `libcef.so`, which is not available to a container agent (see §8). Left-click window drag is a no-op; nothing else I used was affected. `AGENTMUX_CEF_RUNTIME_DIR` points the bundle step at it until the resolver reads the target dir.
4. The first screenshot showed four crash notices. They were old: see §7, "The crash notices". A clean re-run on the corrected image (no `CARGO_TARGET_DIR`, so no symlink and no `AGENTMUX_CEF_RUNTIME_DIR`; `AGENTMUX_UNSAFE_NOSANDBOX=1`; fresh dev home, `--shm-size=2g`) stayed up for over 10 minutes with an agent pane open (§7).

## 7. The crash notices: cause and fix

The first screenshot showed "restarted its window after it stopped unexpectedly" twice, "restarted without graphics acceleration" and "its window process kept crashing (3 times in 60 seconds)". Cause: **CEF's sandbox cannot start in a container** (the host's user-namespace policy blocks it). Without `AGENTMUX_UNSAFE_NOSANDBOX=1` the host process exits at once, the launcher restarts it twice (the second time with graphics acceleration off), then stops after the third exit inside 60 seconds. AgentMux keeps those notices in the dev home and replays them at the next start, so a later launch that is healthy shows them too. My screenshot was that case: an earlier attempt without the variable had crashed in the same dev home, and the run I photographed had the variable set and was up.

Shown by repeating it (charlie, 7f87ebba7, amd64, the prototype image, one container each):

| Run | Result |
|---|---|
| Fresh dev home, `AGENTMUX_UNSAFE_NOSANDBOX=1`, `--shm-size=2g`, a seeded agent pane, a message sent to it | Host process up **10 min 49 s**, same PID throughout, 13 CEF processes steady, no restart or crash line in the log, the agent replied; screenshot at the end shows no notices (status bar uptime 10:49) |
| Fresh dev home, variable **not** set | The sandbox error printed four times (three restarts, then the launcher gave up); the screen goes black |
| Same dev home, variable set, next start | The app comes up and shows exactly the four notices from the first screenshot |
| Fresh dev home, variable set, Docker's default 64 MB `/dev/shm` (AgentMux sets none for container agents) | Up 4 minutes with an agent pane, nothing in `/dev/shm`: shared memory is not a cause |

The GPU is not the cause either: the GPU process exits at start ("Exiting GPU process due to errors during initialization", Vulkan finds no driver) and CEF carries on in software rendering (`GFX SW` in the status bar), as the notices' own "graphics acceleration" wording describes for a machine with none.

**Fix, in the build:** the image's `task dev` entry point (`/usr/local/bin/agent-dev`, a three-line wrapper shipped in the image) sets `AGENTMUX_UNSAFE_NOSANDBOX=1`, starts Xvfb and runs `task dev`; the image documents that the container is the sandbox. A container agent whose `task dev` is started any other way still gets the notices, so the agent's first-start instructions for the developer image name the wrapper. For a user whose container gets the notices anyway, clearing them means deleting the dev home (`.dev-home/`), which the wrapper does when the last run ended in a crash loop.

Two more things the repeat run showed: `npm install` run by `task dev` beside the cargo build exited 146 once on a fresh clone with no `node_modules` (I could not reproduce it: alone it takes 14 s and exits 0), so the wrapper runs `npm ci` first; and a stale Vite from an aborted run holds port 5178 ("Port 5178 is already in use"), so the wrapper stops its own earlier run's processes by PID before starting.

## 8. Gaps (listed, not fixed here)

- **Windows and macOS builds** stay native or on CI: MSVC and the Windows SDK, Xcode and the macOS SDK are not redistributable in a Linux image. A container agent builds and tests the Linux target only.
- **GitHub** goes through AgentMux's App token, not `gh-agent`: clone of a public repo worked unauthenticated; push, PRs and `gh` need the user's Connect with GitHub step.
- **No AWS**: a container agent has no AWS escalation path; AWS work is a native agent's.
- **Patched `libcef.so`** (the 600 MB release asset CI downloads with a token) is not in the image; `task dev` runs on upstream CEF (§6.3). Bundling it needs a token the image cannot carry; the fix is a public asset or a step the user runs with their own sign-in.
- **No GPU**: the window renders in software (`GFX SW` in the status bar). Good enough to screenshot, not for performance work.
- **A shared host's build slot** (charlie's) wraps native cargo calls, not a container's: my container builds ran at full width beside other agents' tests, while the native run queued 18 minutes behind them. Loap-3's CPU limit is what keeps a developer container from doing the same to a host's other users.
- **arm64** is built by the workflow but unmeasured: no arm64 container was run for this spec.
- **Windows and macOS hosts** were not measured. Docker Desktop's VM memory and its bind-mount speed are the two things likely to differ; the numbers above are Linux only.
- **Pull size and time** are estimates until the first publish.

## 9. Build (#11092), in order

0. Taskfile and scripts read `${CARGO_TARGET_DIR:-target}`; `resolve-cef-runtime.sh` too.
1. `rust-toolchain.toml` (if approved), and the Dockerfile reads it.
2. `docker/Dockerfile.agent-developer` (with the `agent-dev` wrapper, §7), the `container-image.yml` job, `release-core.yml` manifest entry, `agent_image_digest.rs` by variant.
3. `container_image.rs`: `versioned_developer_image()`, `resolve_container_image`, `is_agent_base_image` (already true for the tag), `image_blocks_update` by variant; tests for each case in §4.
4. `agent_home_mounts` reads the image's cache-dirs label; labelled volumes; remove an agent's volumes on delete; Clear build cache and sizes; the 20 GiB notice.
5. The Image choice in `AgentCreateForm`.
6. Run the §6 table again on the published image, amd64 and arm64.

**Decisions (approved):** (a) pin Rust in the repo with `rust-toolchain.toml` (§2); (b) tag suffix in the existing package rather than a new package (§3); (c) one variant for v1 (§2); (d) 16 GiB as the memory label (§5).

Agent: Maricon-2@charlie
