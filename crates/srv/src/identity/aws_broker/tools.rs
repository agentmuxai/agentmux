// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The host's private CLIs (`secrets`, `deploy`) for a container agent that has AWS grants.
//!
//! They are private packages (GitHub Packages), so they can't be in the public image and no registry token ever
//! enters a container. Instead srv copies the host's installed packages into one folder under the AgentMux root and
//! mounts it read-only at [`CONTAINER_TOOLS_DIR`]; two small `sh` launchers in `bin/` (on the image's `PATH`) run them
//! with the image's own `node`. A host that doesn't have a CLI installed gets a launcher that says so in one sentence,
//! so the container then has the AWS CLI only. Spec: docs/specs/SPEC_CONTAINER_AGENT_AWS_CREDENTIALS_2026_10_31.md.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use super::policy;

/// Where the folder is mounted, read-only, in the container. Its `bin/` is on the image's `PATH`.
pub const CONTAINER_TOOLS_DIR: &str = "/opt/agentmux/tools";

/// One private CLI: the command the agent types, its npm package, and the script (inside the package) that runs it.
pub struct Tool {
    pub command: &'static str,
    pub package: &'static str,
    pub script: &'static str,
}

pub const TOOLS: [Tool; 2] = [
    Tool { command: "secrets", package: "@a5af/secrets", script: "bin/secrets.js" },
    Tool { command: "deploy", package: "@a5af/deploy-cli", script: "dist/cli.js" },
];

/// What of a package the container runs; the rest (docs, examples) stays on the host.
const KEEP: [&str; 4] = ["package.json", "bin", "dist", "node_modules"];

/// An installed package on the host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub command: &'static str,
    pub version: String,
    pub dir: PathBuf,
}

fn tool(command: &str) -> Option<&'static Tool> {
    TOOLS.iter().find(|t| t.command == command)
}

/// A version that is safe in a folder name and a shell word.
fn is_plain_version(v: &str) -> bool {
    !v.is_empty() && v.len() <= 32 && v.bytes().all(|b| b.is_ascii_alphanumeric() || b".-_+".contains(&b))
}

/// The package folder at `dir` if its `package.json` names `tool`'s package and the script is there.
fn package_at(tool: &'static Tool, dir: &Path) -> Option<Found> {
    let json: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join("package.json")).ok()?).ok()?;
    let version = json.get("version")?.as_str()?.to_string();
    (json.get("name")?.as_str()? == tool.package && is_plain_version(&version) && dir.join(tool.script).is_file())
        .then(|| Found { command: tool.command, version, dir: dir.to_path_buf() })
}

/// Find the host's install of `command`'s package, from where `which` puts the command: on Unix the command is a link
/// into `<prefix>/lib/node_modules/<package>`, on Windows a `.cmd` beside `node_modules\<package>`. No program is run.
pub fn find(command: &str, which: &dyn Fn(&str) -> Option<PathBuf>) -> Option<Found> {
    let tool = tool(command)?;
    let exe = which(command)?;
    let beside = exe.parent().map(|d| d.join("node_modules").join(tool.package));
    let through_link = std::fs::canonicalize(&exe).ok().map(|real| real.ancestors().skip(1).take(4).map(Path::to_path_buf).collect::<Vec<_>>());
    through_link.into_iter().flatten().chain(beside).find_map(|dir| package_at(tool, &dir))
}

fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        // Links are not followed: a package that links outside itself gets nothing there.
        if kind.is_dir() {
            copy_tree(&entry.path(), &to.join(entry.file_name()))?;
        } else if kind.is_file() {
            std::fs::copy(entry.path(), to.join(entry.file_name()))?;
        }
    }
    Ok(())
}

fn copy_package(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in KEEP.iter().map(|e| (e, from.join(e))).filter(|(_, p)| p.exists()) {
        if entry.1.is_dir() {
            copy_tree(&entry.1, &to.join(entry.0))?;
        } else {
            std::fs::copy(&entry.1, to.join(entry.0))?;
        }
    }
    Ok(())
}

static SEQ: AtomicU64 = AtomicU64::new(0);

fn temp_name(stem: &str) -> String {
    format!(".{stem}.{}.{}", std::process::id(), SEQ.fetch_add(1, Ordering::Relaxed))
}

/// Replace `file` with `text` in one step (a container reading the mount never sees half of it), unless it already
/// says that. `executable` sets the mode on Unix hosts; Docker shows a Windows host's files as executable.
pub fn write_replacing(file: &Path, text: &str, executable: bool) -> std::io::Result<()> {
    if std::fs::read_to_string(file).ok().as_deref() == Some(text) {
        return Ok(());
    }
    let temp = file.with_file_name(temp_name(&file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()));
    std::fs::write(&temp, text)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&temp, std::fs::Permissions::from_mode(if executable { 0o755 } else { 0o644 }))?;
    }
    #[cfg(not(unix))]
    let _ = executable;
    std::fs::rename(&temp, file).inspect_err(|_| {
        let _ = std::fs::remove_file(&temp);
    })
}

fn launcher(tool: &Tool, found: Option<&Found>) -> String {
    match found {
        Some(f) => format!(
            "#!/bin/sh\n# Written by AgentMux; read-only. {} {} from this computer, run with the image's node.\nexec node {CONTAINER_TOOLS_DIR}/pkg/{}-{}/{} \"$@\"\n",
            tool.package,
            f.version,
            tool.command,
            f.version,
            tool.script
        ),
        None => format!(
            "#!/bin/sh\necho \"{} is not installed on the computer this container runs on, so this container has the AWS CLI only; a person installs it there (npm i -g {}) and the container picks it up.\" >&2\nexit 127\n",
            tool.command, tool.package
        ),
    }
}

/// Put the tools folder under `root` (`container-tools/`) in order and return it: each found package copied once (a
/// new version gets its own folder, so a running command keeps the files it started from), and a launcher per tool.
pub fn render_into(root: &Path, found: &[Found]) -> std::io::Result<PathBuf> {
    let dir = root.join("container-tools");
    let (bin, pkg) = (dir.join("bin"), dir.join("pkg"));
    std::fs::create_dir_all(&bin)?;
    std::fs::create_dir_all(&pkg)?;
    for tool in &TOOLS {
        let here = found.iter().find(|f| f.command == tool.command);
        if let Some(f) = here {
            let name = format!("{}-{}", f.command, f.version);
            let published = pkg.join(&name);
            if !published.join("package.json").is_file() {
                // Copied beside it and renamed in, so the folder is there whole or not at all.
                let staging = pkg.join(temp_name(&name));
                if let Err(e) = copy_package(&f.dir, &staging).and_then(|_| std::fs::rename(&staging, &published)) {
                    let _ = std::fs::remove_dir_all(&staging);
                    // Another srv instance may have published the same folder first.
                    if !published.join("package.json").is_file() {
                        return Err(e);
                    }
                }
            }
            prune(&pkg, tool.command, &name);
        }
        write_replacing(&bin.join(tool.command), &launcher(tool, here), true)?;
    }
    Ok(dir)
}

/// Remove the folders of `command`'s older versions once they have sat unused for a day, so a command that is still
/// running from one is never pulled out from under it.
fn prune(pkg: &Path, command: &str, keep: &str) {
    let prefix = format!("{command}-");
    let day = std::time::Duration::from_secs(24 * 3600);
    for entry in std::fs::read_dir(pkg).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let old = entry.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age > day);
        if name.starts_with(&prefix) && name != keep && old {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

/// The host folder to mount for the agent `slug`, freshly put in order, or `None`: this host shares no AWS with that
/// agent, or the folder can't be written (the container then has no tools folder, as before).
pub fn config_dir_for(slug: &str) -> Option<String> {
    // The machine running the tests has its own answer; the tests give theirs.
    if cfg!(test) {
        return None;
    }
    if !policy::current().ok().flatten().is_some_and(|p| !p.open_to(slug).is_empty()) {
        return None;
    }
    let root = crate::srv_info::agentmux_root()?;
    let which = |c: &str| which::which(c).ok();
    let found: Vec<Found> = TOOLS.iter().filter_map(|t| find(t.command, &which)).collect();
    match render_into(&root, &found) {
        Ok(dir) => Some(dir.to_string_lossy().into_owned()),
        Err(e) => {
            tracing::warn!(target: "identity", error = %e, "identity.aws: the container's tools folder couldn't be written");
            None
        }
    }
}

#[cfg(test)]
#[path = "tools_tests.rs"]
mod tests;
