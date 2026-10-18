// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What kind of process each Tower row is (`TowerProcess::kind`;
//! SPEC_TOWER_THEME_SURFACE_AND_COLOR_2026_10_17.md §3.2).
//!
//! The first rule that matches wins:
//!
//! 1. AgentMux's own: its task, a process it described, or an AgentMux name.
//! 2. The main process of an agent pane: an agent, whatever its executable.
//! 3. The main process of a terminal pane: a shell.
//! 4. A generic runtime (`node`, `python`, …) inside a pane's task, by its
//!    command line: a package manager or build tool, or an agent CLI's
//!    package. Read here, never sent: only the kind leaves srv.
//! 5. The name, in one table: every provider's CLI, then known names, then
//!    known prefixes.
//! 6. Anything else is an app.
//!
//! Another machine's rows carry names only, so they get rules 1 (by name)
//! and 5 ([`fill_missing`]).

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use crate::backend::rpc_types::{TowerProcessKind as K, TowerProcessRole, TowerSnapshot, TowerTaskKind};

/// A name as the table knows it: lower case, without `.exe`, without a
/// version after a dash (`agentmux-srv-0.59.30-windows.x64` → `agentmux-srv`)
/// or glued on (`python3.12` → `python`, `idea64` → `idea`).
pub fn normalize(name: &str) -> String {
    let mut n = name.trim().to_lowercase();
    if let Some(stripped) = n.strip_suffix(".exe") {
        n = stripped.to_string();
    }
    let parts: Vec<&str> = n.split('-').collect();
    if let Some(at) = parts.iter().position(|p| p.starts_with(|c: char| c.is_ascii_digit())) {
        if at > 0 {
            n = parts[..at].join("-");
        }
    }
    let trimmed = n.trim_end_matches(|c: char| c.is_ascii_digit() || c == '.');
    if trimmed.len() >= 2 && trimmed.len() < n.len() && trimmed.ends_with(|c: char| c.is_ascii_alphabetic()) {
        n = trimmed.to_string();
    }
    n
}

/// Names, after [`normalize`].
const NAMES: &[(K, &[&str])] = &[
    (
        K::Agents,
        &[
            "ollama", "ollama app", "ollama_llama_server", "llama-server", "llama-cli", "lmstudio", "lm studio",
            "lms", "koboldcpp", "jan", "gpt4all", "cursor-agent", "amp", "chatgpt",
        ],
    ),
    (
        K::Shells,
        &[
            // Shells and terminal hosts.
            "bash", "sh", "zsh", "fish", "nu", "dash", "ksh", "tcsh", "csh", "elvish", "xonsh", "pwsh",
            "powershell", "cmd", "conhost", "openconsole", "windowsterminal", "wt", "tmux", "screen", "login",
            "wezterm", "wezterm-gui", "alacritty", "kitty", "iterm2", "terminal", "gnome-terminal-server",
            "konsole", "xterm", "ghostty", "warp",
            // Remote sessions.
            "ssh", "sshd", "mosh-client", "mosh-server", "sftp", "scp", "telnet",
            // Commands run in them.
            "grep", "rg", "ag", "sed", "awk", "gawk", "find", "fd", "xargs", "head", "tail", "cat", "less",
            "more", "sort", "uniq", "wc", "cut", "tr", "tee", "jq", "yq", "fold", "ls", "cp", "mv", "rm",
            "mkdir", "touch", "chmod", "sleep", "watch", "timeout", "env", "curl", "wget", "http", "nc", "ping",
            "tar", "zip", "unzip", "7z", "gzip", "xz", "du", "df", "ps", "top", "htop", "btop", "kill", "which",
            "where", "findstr", "clip", "tasklist", "taskkill", "netstat", "ipconfig", "whoami", "hostname",
            "date", "diff", "patch", "rsync", "fzf", "bat", "eza", "tree",
        ],
    ),
    (
        K::Builds,
        &[
            // Compilers, linkers, build systems.
            "cargo", "rustc", "rustup", "rustfmt", "clippy-driver", "cargo-clippy", "cargo-nextest", "rustdoc",
            "cc", "c++", "cl", "link", "lib", "ml", "rc", "mt", "lld", "lld-link", "ld", "ld.lld", "gold", "ar",
            "ranlib", "nm", "strip", "objcopy", "clang", "clang++", "clang-cl", "gcc", "g++", "cc1", "cc1plus",
            "as", "collect2", "make", "mingw32-make", "gmake", "cmake", "ninja", "meson", "bazel", "bazelisk",
            "buck2", "msbuild", "vbcscompiler", "csc", "tsc", "esbuild", "swc", "go", "compile", "asm", "gradle",
            "mvn", "ant", "javac", "kotlinc", "scalac", "sbt", "zig", "nim", "ghc", "dune", "swiftc",
            "swift-frontend", "xcodebuild", "xcrun", "protoc", "flatc", "wasm-pack", "wasm-opt", "wasm-bindgen",
            "trunk", "task", "just",
            // Package managers.
            "npm", "npx", "pnpm", "yarn", "pip", "uv", "uvx", "poetry", "pipx", "conda", "mamba", "gem", "bundle",
            "bundler", "composer", "brew", "apt", "apt-get", "dpkg", "dnf", "yum", "pacman", "winget", "choco",
            "scoop", "nuget", "vcpkg",
            // Version control.
            "git", "git-remote-https", "git-lfs", "git-credential-manager", "gh", "gh-agent", "glab", "hg", "svn",
            "jj",
            // Test runners with their own executable.
            "pytest", "cargo-test",
        ],
    ),
    (
        K::Runtimes,
        &[
            "node", "nodejs", "deno", "bun", "python", "pythonw", "py", "ruby", "rubyw", "java", "javaw", "dotnet",
            "php", "php-cgi", "perl", "lua", "luajit", "r", "rscript", "julia", "erl", "beam.smp", "elixir", "iex",
            "mono", "wasmtime", "wasmer",
            // Language servers.
            "rust-analyzer", "gopls", "clangd", "pylsp", "omnisharp", "sourcekit-lsp", "lua-language-server",
            "zls", "marksman", "taplo", "eslint_d",
            // Databases and local servers.
            "postgres", "postmaster", "mysqld", "mariadbd", "mongod", "mongos", "redis-server", "valkey-server",
            "sqlservr", "memcached", "etcd", "clickhouse", "influxd", "nginx", "httpd", "apache2", "caddy",
            "traefik", "envoy", "haproxy", "php-fpm", "iisexpress", "w3wp",
        ],
    ),
    (
        K::Browsers,
        &[
            "chrome", "msedge", "firefox", "brave", "opera", "vivaldi", "safari", "chromium", "chromium-browser",
            "msedgewebview", "iexplore", "arc", "plugin-container", "chrome-native-host", "safariviewservice",
        ],
    ),
    (
        K::Containers,
        &[
            "docker", "dockerd", "docker desktop", "docker-desktop", "containerd", "runc", "podman",
            "podman-machine", "buildkitd", "minikube", "colima", "limactl", "vboxheadless", "vboxsvc",
            "vboxnetdhcp", "vmmem", "vmmemwsl", "vmwp", "vmcompute", "vmms", "wsl", "wslhost", "wslrelay",
            "wslservice", "hyperkit", "vpnkit", "utm", "virtualbox", "vmnat", "vmnetdhcp",
        ],
    ),
    (
        K::Editors,
        &[
            "code", "code - insiders", "cursor", "windsurf", "zed", "sublime_text", "subl", "atom", "notepad++",
            "notepad", "idea", "pycharm", "webstorm", "rider", "goland", "clion", "rustrover", "datagrip",
            "phpstorm", "studio", "devenv", "xcode", "nvim", "vim", "gvim", "emacs", "nano", "hx", "helix", "kak",
            "micro", "fleet",
        ],
    ),
    (
        K::System,
        &[
            // Windows.
            "system", "registry", "idle", "secure system", "memory compression", "smss", "csrss", "wininit",
            "winlogon", "services", "lsass", "lsaiso", "svchost", "dwm", "explorer", "runtimebroker", "dllhost",
            "taskhostw", "sihost", "fontdrvhost", "ctfmon", "searchhost", "searchindexer", "searchprotocolhost",
            "searchfilterhost", "wmiprvse", "spoolsv", "audiodg", "textinputhost", "startmenuexperiencehost",
            "shellexperiencehost", "shellhost", "applicationframehost", "backgroundtaskhost", "unsecapp", "wudfhost",
            "dashost", "aggregatorhost", "smartscreen", "securityhealthservice", "securityhealthsystray",
            "securityhealthhost", "sechealthui", "msmpeng", "mpdefendercoreservice", "nissrv", "sgrmbroker",
            "sysmon", "lsm", "ngciso", "widgetservice", "widgetboard", "phoneexperiencehost", "lockapp",
            "useroobebroker", "crossdeviceresume", "tabtip", "mousocoreworker", "trustedinstaller", "tiworker",
            "compattelrunner", "msiexec", "werfault", "ipoverusbsvc", "defendersessionhelper",
            "midisrv", "monotificationux", "apphostregistrationverifier",
            // macOS.
            "kernel_task", "launchd", "windowserver", "mds", "mds_stores", "mdworker", "mdworker_shared",
            "coreaudiod", "loginwindow", "distnoted", "cfprefsd", "trustd", "syspolicyd", "opendirectoryd",
            "configd", "powerd", "logd", "notifyd", "coreservicesd", "bluetoothd", "airportd", "controlcenter",
            "systemuiserver", "usereventagent", "secd", "nsurlsessiond", "sharingd", "rapportd", "finder", "dock",
            "spotlight", "corespotlightd",
            // Linux.
            "init", "kthreadd", "kswapd", "dbus-daemon", "dbus-broker", "xorg", "xwayland", "gnome-shell",
            "plasmashell", "kwin_x11", "kwin_wayland", "pipewire", "wireplumber", "pulseaudio", "networkmanager",
            "polkitd", "udisksd", "upowerd", "cron", "crond", "rsyslogd", "snapd", "agetty", "kauditd",
            "khugepaged", "oom_reaper", "gdm", "sddm", "lightdm",
        ],
    ),
];

/// Prefixes, after [`normalize`]; the longest that matches wins.
const PREFIXES: &[(K, &[&str])] = &[
    (K::Agentmux, &["agentmux"]),
    (K::Browsers, &["google chrome", "microsoft edge", "firefox", "brave browser", "com.apple.webkit.", "msedgewebview"]),
    (K::Containers, &["com.docker.", "containerd-shim", "qemu-", "qemu-system", "vmware", "virtualbox", "vbox", "lima", "com.apple.virtualization."]),
    (K::Editors, &["code helper", "cursor helper", "jetbrains"]),
    (
        K::System,
        &[
            "systemd", "kworker/", "ksoftirqd/", "migration/", "rcu_", "irq/", "jbd2/", "watchdog/", "com.apple.",
            "nvcontainer", "nvdisplay.container", "nvidia", "amd3dvcache", "amdrs", "amdppkg", "amdfend", "amdow",
            "amdappcompat", "atiesrxx", "atieclxx", "radeonsoftware", "rtk", "realtek", "igfx", "intelcp", "logi_",
            "logioptionsplus", "lghub", "razer", "asus", "dtsapo", "nahimic", "securityhealth", "mpdefender",
            "searchapp", "microsoft.photos", "microsoftstartfeedprovider",
        ],
    ),
];

static BY_NAME: LazyLock<HashMap<&'static str, K>> = LazyLock::new(|| {
    let mut m = HashMap::new();
    for (kind, names) in NAMES {
        for n in *names {
            m.insert(*n, *kind);
        }
    }
    m
});

/// Every provider's CLI, from the registry (`providers::all_providers`), so a
/// new provider is an agent here with no edit.
static AGENT_CLIS: LazyLock<HashSet<String>> =
    LazyLock::new(|| crate::backend::providers::all_providers().map(|p| normalize(p.cli_command)).collect());

/// Every provider's npm packages: what a `node` running an agent CLI has in
/// its command line.
static AGENT_PACKAGES: LazyLock<Vec<String>> = LazyLock::new(|| {
    let mut out = Vec::new();
    for p in crate::backend::providers::all_providers() {
        if !p.npm_package.is_empty() {
            out.push(p.npm_package.to_ascii_lowercase());
        }
        for (pkg, _) in p.companion_npm_packages() {
            out.push(pkg.to_ascii_lowercase());
        }
    }
    out
});

/// What `name` alone says (rules 1 by name, and 5).
pub fn by_name(name: &str) -> K {
    let n = normalize(name);
    if AGENT_CLIS.contains(&n) {
        return K::Agents;
    }
    if let Some(&kind) = BY_NAME.get(n.as_str()) {
        return kind;
    }
    PREFIXES
        .iter()
        .flat_map(|(kind, prefixes)| prefixes.iter().map(move |p| (*kind, *p)))
        .filter(|(_, p)| n.starts_with(p))
        .max_by_key(|(_, p)| p.len())
        .map(|(kind, _)| kind)
        .unwrap_or(K::Apps)
}

/// A runtime whose command line says more than its name (rule 4).
pub fn is_generic_runtime(name: &str) -> bool {
    matches!(
        normalize(name).as_str(),
        "node" | "nodejs" | "python" | "pythonw" | "py" | "java" | "javaw" | "dotnet" | "deno" | "bun" | "ruby" | "php"
    )
}

/// Build and package tools a runtime runs, by their npm package: matched only
/// as the package directory of the script it runs (`/node_modules/<pkg>/`) or
/// as a `node_modules/.bin/<tool>` shim, never as any part of a path, so a
/// project at `~/turbo-app` is not a build.
const BUILD_PACKAGES: &[&str] = &[
    "npm", "pnpm", "yarn", "corepack", "vite", "webpack", "webpack-cli", "jest", "vitest", "mocha",
    "esbuild", "rollup", "turbo", "playwright", "@playwright/test", "eslint", "prettier", "@biomejs/biome", "nx",
    "parcel", "gulp", "grunt", "lerna", "@changesets/cli", "@angular/cli",
];

/// The same tools as a `node_modules/.bin/` shim names them.
const BUILD_BINS: &[&str] = &[
    "npm", "npx", "pnpm", "yarn", "vite", "webpack", "tsc", "jest", "vitest", "mocha", "esbuild", "rollup", "turbo",
    "playwright", "eslint", "prettier", "biome", "nx", "parcel", "gulp", "grunt", "lerna", "changeset", "ng",
];

/// Python modules (`-m <module>`) and scripts that build, test or install.
const BUILD_PYTHON: &[&str] = &[
    "pytest", "pip", "pip3", "build", "unittest", "tox", "nox", "poetry", "pdm", "hatch", "twine", "maturin", "mypy",
    "ruff", "setup.py",
];

/// A command line's words, double quotes respected: `"C:\Program Files\x"`
/// is one word.
fn words(command_line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut word = String::new();
    let mut quoted = false;
    for ch in command_line.chars() {
        match ch {
            '"' => quoted = !quoted,
            c if c.is_whitespace() && !quoted => {
                if !word.is_empty() {
                    out.push(std::mem::take(&mut word));
                }
            }
            c => word.push(c),
        }
    }
    if !word.is_empty() {
        out.push(word);
    }
    out
}

/// The last part of a path, after `/`.
fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// What a generic runtime's command line says (rule 4): `None` keeps it a
/// runtime. Only what it runs counts, never any word of a path: the script's
/// package directory, a `.bin` shim, Python's `-m` module or script name, a
/// JVM's launcher class, `dotnet`'s verb. Pure: the caller reads the command
/// line.
pub fn by_command_line(command_line: &str) -> Option<K> {
    let words: Vec<String> = words(&command_line.to_ascii_lowercase().replace('\\', "/"));
    let program = basename(words.first()?).trim_end_matches(".exe").to_string();
    let rest = &words[1..];
    // The first word that isn't a flag: the script a runtime runs.
    let script = rest.iter().find(|w| !w.starts_with('-')).map(String::as_str).unwrap_or("");
    let in_package = |pkg: &str| script.contains(&format!("/node_modules/{pkg}/"));

    if AGENT_PACKAGES.iter().any(|p| in_package(p)) {
        return Some(K::Agents);
    }
    match normalize(&program).as_str() {
        "node" | "nodejs" | "deno" | "bun" => {
            let shim = script
                .rsplit_once("/node_modules/.bin/")
                .is_some_and(|(_, tool)| BUILD_BINS.contains(&tool.trim_end_matches(".js")));
            let yarn_release = script.contains("/.yarn/releases/");
            // TypeScript's package also holds its language server: only `tsc` builds.
            let tsc = script.contains("/node_modules/typescript/bin/tsc")
                || script.contains("/node_modules/typescript/lib/tsc.js");
            (BUILD_PACKAGES.iter().any(|p| in_package(p)) || shim || yarn_release || tsc).then_some(K::Builds)
        }
        "python" | "pythonw" | "py" => {
            let module = rest.iter().position(|w| w == "-m").and_then(|i| rest.get(i + 1));
            let named = module.map(String::as_str).unwrap_or_else(|| basename(script));
            let named = named.trim_end_matches(".exe");
            BUILD_PYTHON.contains(&named).then_some(K::Builds)
        }
        // Gradle's daemon and Maven run in a JVM: their launcher classes say so.
        "java" | "javaw" => rest
            .iter()
            .any(|w| w.starts_with("org.gradle.") || w.contains("org.codehaus.plexus.classworlds") || w.contains("plexus-classworlds"))
            .then_some(K::Builds),
        "dotnet" => {
            let verb = matches!(script, "build" | "test" | "restore" | "publish" | "pack" | "msbuild");
            let tool = matches!(basename(script), "msbuild.dll" | "vbcscompiler.dll");
            (verb || tool).then_some(K::Builds)
        }
        _ => None,
    }
}

/// Where a process sits, for rules 1–3.
#[derive(Debug, Clone, Copy, Default)]
pub struct Context {
    /// Its part in a pane's task, and that task's kind.
    pub task: Option<(TowerProcessRole, TowerTaskKind)>,
    /// One of AgentMux's own, by its grouping or a description.
    pub agentmux: bool,
}

/// The kind, from where the process sits (rules 1–3), else `named`: what its
/// name, or its command line, said ([`by_name`], [`by_command_line`]).
pub fn classify(ctx: Context, named: K) -> K {
    if ctx.agentmux || named == K::Agentmux {
        return K::Agentmux;
    }
    match ctx.task {
        Some((_, TowerTaskKind::Agentmux)) => K::Agentmux,
        Some((TowerProcessRole::Main, TowerTaskKind::Agent)) => K::Agents,
        Some((TowerProcessRole::Main, TowerTaskKind::Terminal)) => K::Shells,
        _ => named,
    }
}

/// Fill the kind of every row that has none: another machine's frame (names
/// only), or a paired computer's AgentMux that predates kinds, whose tasks
/// still say what each process is to its pane, so rules 1–3 apply there too.
pub fn fill_missing(snap: &mut TowerSnapshot) {
    use crate::backend::rpc_types::TowerProcess;
    let kind_of = |p: &TowerProcess, task: Option<TowerTaskKind>, role: Option<TowerProcessRole>| {
        let ctx = Context {
            task: task.map(|t| (role.unwrap_or(TowerProcessRole::Started), t)),
            agentmux: p.detail.is_some(),
        };
        classify(ctx, by_name(&p.name))
    };
    // A host row names its task; its role is on the task's own row for it.
    let mut placed: HashMap<String, (TowerTaskKind, Option<TowerProcessRole>)> = HashMap::new();
    for t in &mut snap.tasks {
        for p in t.processes.iter_mut().chain(t.exited.iter_mut().flatten()) {
            placed.insert(p.id.clone(), (t.kind, p.role));
            if p.kind.is_none() {
                p.kind = Some(kind_of(p, Some(t.kind), p.role));
            }
        }
    }
    let task_kinds: HashMap<String, TowerTaskKind> = snap.tasks.iter().map(|t| (t.id.clone(), t.kind)).collect();
    if let Some(host) = snap.host.as_mut() {
        for p in &mut host.processes {
            if p.kind.is_some() {
                continue;
            }
            let (task, role) = match placed.get(&p.id) {
                Some(&(kind, role)) => (Some(kind), role),
                None => (p.task.as_ref().and_then(|t| task_kinds.get(t)).copied(), None),
            };
            p.kind = Some(kind_of(p, task, role));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_normalized_before_lookup() {
        assert_eq!(normalize("agentmux-srv-0.59.30-windows.x64.exe"), "agentmux-srv");
        assert_eq!(normalize("agentmux-0.59.28.exe"), "agentmux");
        assert_eq!(normalize("python3.12"), "python");
        assert_eq!(normalize("idea64.exe"), "idea");
        assert_eq!(normalize("MsEdgeWebView2.exe"), "msedgewebview");
        assert_eq!(normalize("x86_64-w64-mingw32-gcc"), "x86_64-w64-mingw32-gcc");
        assert_eq!(normalize("7z.exe"), "7z");
        assert_eq!(normalize("Google Chrome Helper (Renderer)"), "google chrome helper (renderer)");
    }

    #[test]
    fn names_from_each_os_get_their_kind() {
        let cases = [
            ("svchost.exe", K::System),
            ("csrss.exe", K::System),
            ("kernel_task", K::System),
            ("WindowServer", K::System),
            ("kworker/0:1H-events", K::System),
            ("systemd-journald", K::System),
            ("nvcontainer.exe", K::System),
            ("conhost.exe", K::Shells),
            ("bash", K::Shells),
            ("pwsh.exe", K::Shells),
            ("grep", K::Shells),
            ("cargo.exe", K::Builds),
            ("rustc", K::Builds),
            ("git.exe", K::Builds),
            ("gh-agent", K::Builds),
            ("node.exe", K::Runtimes),
            ("python3.11", K::Runtimes),
            ("rust-analyzer", K::Runtimes),
            ("postgres", K::Runtimes),
            ("chrome.exe", K::Browsers),
            ("msedgewebview2.exe", K::Browsers),
            ("Google Chrome Helper (Renderer)", K::Browsers),
            ("com.apple.WebKit.WebContent", K::Browsers),
            ("vmmemWSL", K::Containers),
            ("com.docker.backend.exe", K::Containers),
            ("Docker Desktop.exe", K::Containers),
            ("qemu-system-x86_64", K::Containers),
            ("Code.exe", K::Editors),
            ("Code Helper (Renderer)", K::Editors),
            ("idea64.exe", K::Editors),
            ("ollama.exe", K::Agents),
            ("agentmux-cef.exe", K::Agentmux),
            ("agentmux-srv-0.59.30-windows.x64.exe", K::Agentmux),
            ("Discord.exe", K::Apps),
            ("KeePass.exe", K::Apps),
            ("CodeMeter.exe", K::Apps),
            ("", K::Apps),
        ];
        for (name, want) in cases {
            assert_eq!(by_name(name), want, "{name}");
        }
    }

    #[test]
    fn every_provider_cli_is_an_agent() {
        for p in crate::backend::providers::all_providers() {
            assert_eq!(by_name(p.cli_command), K::Agents, "{}", p.cli_command);
            assert_eq!(by_name(&format!("{}.exe", p.cli_command)), K::Agents, "{}", p.cli_command);
        }
    }

    #[test]
    fn a_runtime_command_line_says_build_or_agent() {
        let npm = r#""C:\Program Files\nodejs\node.exe" "C:\Program Files\nodejs\node_modules\npm\bin\npm-cli.js" install"#;
        assert_eq!(by_command_line(npm), Some(K::Builds));
        assert_eq!(by_command_line("node /repo/node_modules/vite/bin/vite.js --port 5173"), Some(K::Builds));
        assert_eq!(by_command_line("node --max-old-space-size=4096 /repo/node_modules/.bin/vitest run"), Some(K::Builds));
        assert_eq!(by_command_line("node /repo/.yarn/releases/yarn-4.1.0.cjs install"), Some(K::Builds));
        assert_eq!(by_command_line("node /repo/node_modules/typescript/bin/tsc -p ."), Some(K::Builds));
        assert_eq!(by_command_line("python -m pytest tests/"), Some(K::Builds));
        assert_eq!(by_command_line("python3 /repo/.venv/bin/pytest -q"), Some(K::Builds));
        assert_eq!(by_command_line("python setup.py build"), Some(K::Builds));
        assert_eq!(by_command_line(r#""C:\dotnet\dotnet.exe" build app.sln"#), Some(K::Builds));
        let gradle = "java -Xmx2g -cp /home/u/.gradle/wrapper/dists/gradle-8.5/lib/gradle-launcher-8.5.jar \
                      org.gradle.launcher.daemon.bootstrap.GradleDaemon 8.5";
        assert_eq!(by_command_line(gradle), Some(K::Builds));
        let gemini = r"C:\Users\x\AppData\Roaming\npm\node_modules\@google\gemini-cli\dist\index.js";
        assert_eq!(by_command_line(&format!("node {gemini}")), Some(K::Agents));
        let claude = "/usr/lib/node_modules/@anthropic-ai/claude-code/cli.js";
        assert_eq!(by_command_line(&format!("node --no-warnings {claude} --print")), Some(K::Agents));
        // A server or a language server stays a runtime.
        assert_eq!(by_command_line("node server.js"), None);
        assert_eq!(by_command_line("node /x/node_modules/typescript/lib/tsserver.js"), None);
        assert_eq!(by_command_line("python manage.py runserver"), None);
        assert!(is_generic_runtime("node.exe") && is_generic_runtime("python3.12") && !is_generic_runtime("cargo"));
    }

    #[test]
    fn a_tool_name_in_a_project_path_is_not_a_build() {
        // A tool's name as any part of a path proves nothing: only what runs counts.
        for c in [
            "node /home/u/yarn-site/server.js",
            "node /home/u/turbo-app/server.js",
            "node /srv/jest-dashboard/index.js",
            "node /work/webpack-notes/serve.js --gradle",
            "node /work/nx/app.js",
            "python /home/u/setup.py.d/serve.py",
            "python /home/u/pytest-reports/app.py",
            "java -jar /opt/gradle-dashboard/app.jar",
        ] {
            assert_eq!(by_command_line(c), None, "{c}");
        }
    }

    #[test]
    fn where_a_process_sits_comes_before_its_name() {
        let main_of = |kind| Context { task: Some((TowerProcessRole::Main, kind)), agentmux: false };
        // An agent CLI run by a generic `node`: still the agent pane's agent.
        assert_eq!(classify(main_of(TowerTaskKind::Agent), K::Runtimes), K::Agents);
        assert_eq!(classify(main_of(TowerTaskKind::Terminal), K::Apps), K::Shells);
        assert_eq!(classify(main_of(TowerTaskKind::Agentmux), K::Apps), K::Agentmux);
        let started = Context { task: Some((TowerProcessRole::Started, TowerTaskKind::Agent)), agentmux: false };
        assert_eq!(classify(started, K::Builds), K::Builds, "what an agent runs keeps its own kind");
        assert_eq!(classify(Context { task: None, agentmux: true }, K::Browsers), K::Agentmux, "a renderer is AgentMux's");
        assert_eq!(classify(Context::default(), K::Agentmux), K::Agentmux);
    }

    #[test]
    fn an_older_paired_snapshot_is_filled_from_its_tasks() {
        use crate::backend::rpc_types::{TowerHost, TowerProcess, TowerTask};
        let row = |id: &str, name: &str, role: Option<TowerProcessRole>, task: Option<&str>| TowerProcess {
            id: id.into(),
            name: name.into(),
            role,
            task: task.map(str::to_string),
            ..Default::default()
        };
        let task = |id: &str, kind, processes| TowerTask {
            id: id.into(),
            kind,
            label: id.into(),
            tracking: "high".into(),
            cpu: None,
            cpu_account: false,
            mem: 0,
            processes,
            exited: None,
        };
        let mut snap = TowerSnapshot {
            ts_ms: 0,
            hostname: "studio".into(),
            os: "macos".into(),
            cpu_count: 1,
            memory_metric: String::new(),
            interval_ms: 2000,
            remote: true,
            tasks: vec![
                task("agent-a", TowerTaskKind::Agent, vec![
                    row("1:1", "node", Some(TowerProcessRole::Main), None),
                    row("2:1", "cargo", Some(TowerProcessRole::Started), None),
                ]),
                task("agentmux", TowerTaskKind::Agentmux, vec![row("3:1", "helper-x", Some(TowerProcessRole::Main), None)]),
            ],
            machine: None,
            host: Some(TowerHost {
                processes: vec![
                    row("1:1", "node", None, Some("agent-a")),
                    row("3:1", "helper-x", None, Some("agentmux")),
                    row("9:1", "node", None, None),
                ],
                unmeasured: 0,
                cpu: None,
                mem: 0,
                total: 3,
                matched: 3,
            }),
        };
        fill_missing(&mut snap);
        let kinds = |ps: &[TowerProcess]| ps.iter().map(|p| p.kind).collect::<Vec<_>>();
        assert_eq!(kinds(&snap.tasks[0].processes), vec![Some(K::Agents), Some(K::Builds)], "the agent's own node is the agent");
        assert_eq!(kinds(&snap.tasks[1].processes), vec![Some(K::Agentmux)]);
        assert_eq!(
            kinds(&snap.host.unwrap().processes),
            vec![Some(K::Agents), Some(K::Agentmux), Some(K::Runtimes)],
            "host rows take their task's reading; a node of no task is a runtime"
        );
    }

    #[test]
    fn a_frame_without_kinds_is_filled_by_name() {
        use crate::backend::rpc_types::{TowerHost, TowerProcess};
        let mut snap = TowerSnapshot {
            ts_ms: 0,
            hostname: "h".into(),
            os: "linux".into(),
            cpu_count: 1,
            memory_metric: String::new(),
            interval_ms: 2000,
            remote: true,
            tasks: Vec::new(),
            machine: None,
            host: Some(TowerHost {
                processes: vec![
                    TowerProcess { name: "cargo".into(), ..Default::default() },
                    TowerProcess { name: "node".into(), kind: Some(K::Builds), ..Default::default() },
                ],
                unmeasured: 0,
                cpu: None,
                mem: 0,
                total: 2,
                matched: 2,
            }),
        };
        fill_missing(&mut snap);
        let kinds: Vec<_> = snap.host.unwrap().processes.into_iter().map(|p| p.kind).collect();
        assert_eq!(kinds, vec![Some(K::Builds), Some(K::Builds)], "a kind already set is kept");
    }
}
