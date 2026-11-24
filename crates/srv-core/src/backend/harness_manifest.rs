// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The manifest of a custom harness: `harness.json`, one per harness folder.
//!
//! Parsing and validation only; nothing here reads the disk or runs a program.
//! A broken rule refuses the manifest and names the field, so the person can
//! fix it. SPEC_BYO_HARNESS_CUSTOM_ACP_MANIFEST_2026_11_12.md §3.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

/// The only manifest version this build reads.
pub const MANIFEST_VERSION: u32 = 1;
/// A custom harness id starts with this, so it can never shadow a built-in.
pub const ID_PREFIX: &str = "custom-";

const MAX_ID_LEN: usize = 48;
const MAX_NAME_LEN: usize = 64;
const MAX_ARGS: usize = 64;
const MAX_ARG_LEN: usize = 4096;
const MAX_ENV: usize = 32;
const MAX_VENDORS: usize = 8;
const MAX_FILE_NAME_LEN: usize = 64;

/// A broken rule: the manifest field and what is wrong with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestError {
    pub field: String,
    pub reason: String,
}

impl fmt::Display for ManifestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "harness.json: {}: {}", self.field, self.reason)
    }
}

impl std::error::Error for ManifestError {}

fn bad(field: &str, reason: impl Into<String>) -> ManifestError {
    ManifestError { field: field.to_string(), reason: reason.into() }
}

/// How the harness signs in. A secret is never in the manifest: a key is a
/// Connectors account brokered into the variable per use, a config dir is the
/// per-identity folder.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Auth {
    #[default]
    None,
    Key {
        #[serde(rename = "envVar")]
        env_var: String,
    },
    ConfigDir {
        #[serde(rename = "envVar")]
        env_var: String,
    },
}

/// One platform's download: an https address and the SHA-256 of what it serves.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DownloadAsset {
    pub url: String,
    pub sha256: String,
}

/// Where the program comes from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Install {
    /// Already on this machine: native agents only.
    Path,
    /// A pinned npm package, installed beside the built-in ones.
    Npm { package: String, version: String },
    /// A download per platform, keyed `<os>-<arch>` (`windows-x86_64`,
    /// `macos-aarch64`, `linux-x86_64`), each with its SHA-256.
    Download { urls: BTreeMap<String, DownloadAsset> },
}

/// `harness.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HarnessManifest {
    pub manifest_version: u32,
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub icon: Option<String>,
    pub protocol: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub auth: Auth,
    #[serde(default)]
    pub startup_instructions: Option<String>,
    #[serde(default)]
    pub vendors: Vec<String>,
    #[serde(default)]
    pub base_url_env_var: Option<String>,
    pub install: Install,
}

impl HarnessManifest {
    /// Parse and validate. A JSON error names where it stopped; a broken rule
    /// names its field.
    pub fn parse(text: &str) -> Result<Self, ManifestError> {
        let manifest: HarnessManifest =
            serde_json::from_str(text).map_err(|e| bad("(file)", format!("not a valid manifest: {e}")))?;
        manifest.validate()?;
        Ok(manifest)
    }

    /// Check every rule of the manifest table, first broken one wins.
    pub fn validate(&self) -> Result<(), ManifestError> {
        if self.manifest_version != MANIFEST_VERSION {
            return Err(bad("manifestVersion", format!("must be {MANIFEST_VERSION}")));
        }
        check_id(&self.id)?;
        check_name(&self.name)?;
        if let Some(icon) = &self.icon {
            check_icon(icon)?;
        }
        if self.protocol != "acp" {
            return Err(bad("protocol", "must be \"acp\""));
        }
        check_command(&self.command)?;
        check_args(&self.args)?;
        check_env(&self.env)?;
        check_auth(&self.auth, &self.env)?;
        if let Some(file) = &self.startup_instructions {
            check_file_name("startupInstructions", file)?;
        }
        check_vendors(&self.vendors)?;
        if let Some(var) = &self.base_url_env_var {
            check_env_name("baseUrlEnvVar", var)?;
            if self.env.contains_key(var) {
                return Err(bad("baseUrlEnvVar", "is also set in env"));
            }
        }
        check_install(&self.install, &self.command)
    }
}

fn has_control(s: &str) -> bool {
    s.chars().any(|c| c.is_control())
}

fn check_id(id: &str) -> Result<(), ManifestError> {
    let Some(rest) = id.strip_prefix(ID_PREFIX) else {
        return Err(bad("id", format!("must start with \"{ID_PREFIX}\"")));
    };
    if rest.is_empty() || id.len() > MAX_ID_LEN {
        return Err(bad("id", format!("a name after \"{ID_PREFIX}\", at most {MAX_ID_LEN} characters in all")));
    }
    if !rest.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
        return Err(bad("id", "lowercase letters, digits and - only"));
    }
    if rest.starts_with('-') || rest.ends_with('-') {
        return Err(bad("id", "the name cannot start or end with -"));
    }
    Ok(())
}

fn check_name(name: &str) -> Result<(), ManifestError> {
    if name.trim().is_empty() || name.chars().count() > MAX_NAME_LEN || has_control(name) {
        return Err(bad("name", format!("1 to {MAX_NAME_LEN} characters, no control characters")));
    }
    Ok(())
}

/// A glyph name (`bot`) or a file in the harness folder (`logo.png`, `logo.svg`).
fn check_icon(icon: &str) -> Result<(), ManifestError> {
    if icon.contains('.') {
        check_file_name("icon", icon)?;
        let lower = icon.to_ascii_lowercase();
        if !(lower.ends_with(".png") || lower.ends_with(".svg")) {
            return Err(bad("icon", "an image file must be .png or .svg"));
        }
        return Ok(());
    }
    if icon.is_empty() || icon.len() > 32 || !icon.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
        return Err(bad("icon", "a glyph name (lowercase letters, digits, -) or a .png or .svg file name"));
    }
    Ok(())
}

/// A plain file name: no separator, no `..`, nothing that names a drive or a stream.
fn check_file_name(field: &str, name: &str) -> Result<(), ManifestError> {
    let plain = !name.is_empty()
        && name.len() <= MAX_FILE_NAME_LEN
        && name != "."
        && !name.contains("..")
        && !name.chars().any(|c| matches!(c, '/' | '\\' | ':' | '\0') || c.is_control());
    if plain {
        Ok(())
    } else {
        Err(bad(field, "a plain file name: no folder, no .. and no drive"))
    }
}

/// An absolute path, or a bare program name. Never a command string.
fn check_command(command: &str) -> Result<(), ManifestError> {
    if command.is_empty() || has_control(command) {
        return Err(bad("command", "a program, with no control characters"));
    }
    if std::path::Path::new(command).is_absolute() {
        return Ok(());
    }
    let bare = command.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    if bare && !command.starts_with('.') {
        Ok(())
    } else {
        Err(bad("command", "an absolute path or a bare program name; put the arguments in args, not in command"))
    }
}

fn check_args(args: &[String]) -> Result<(), ManifestError> {
    if args.len() > MAX_ARGS {
        return Err(bad("args", format!("at most {MAX_ARGS} arguments")));
    }
    for (i, arg) in args.iter().enumerate() {
        if arg.len() > MAX_ARG_LEN || arg.contains('\0') {
            return Err(bad(&format!("args[{i}]"), format!("at most {MAX_ARG_LEN} bytes, no NUL")));
        }
    }
    Ok(())
}

/// A variable name the manifest may set or name: not AgentMux's own, not PATH.
fn check_env_name(field: &str, name: &str) -> Result<(), ManifestError> {
    let mut chars = name.chars();
    let starts_ok = chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_');
    if !starts_ok || !chars.all(|c| c.is_ascii_alphanumeric() || c == '_') || name.len() > 64 {
        return Err(bad(field, "a variable name: letters, digits and _, not starting with a digit"));
    }
    if name.to_ascii_uppercase().starts_with("AGENTMUX_") {
        return Err(bad(field, "AGENTMUX_* variables belong to AgentMux"));
    }
    if name.eq_ignore_ascii_case("PATH") {
        return Err(bad(field, "PATH cannot be set by a manifest"));
    }
    Ok(())
}

fn check_env(env: &BTreeMap<String, String>) -> Result<(), ManifestError> {
    if env.len() > MAX_ENV {
        return Err(bad("env", format!("at most {MAX_ENV} variables")));
    }
    for (name, value) in env {
        check_env_name(&format!("env.{name}"), name)?;
        if value.contains('\0') || value.len() > MAX_ARG_LEN {
            return Err(bad(&format!("env.{name}"), format!("at most {MAX_ARG_LEN} bytes, no NUL")));
        }
    }
    Ok(())
}

fn check_auth(auth: &Auth, env: &BTreeMap<String, String>) -> Result<(), ManifestError> {
    let var = match auth {
        Auth::None => return Ok(()),
        Auth::Key { env_var } | Auth::ConfigDir { env_var } => env_var,
    };
    check_env_name("auth.envVar", var)?;
    if env.contains_key(var) {
        return Err(bad("auth.envVar", "is also set in env: a sign-in value is never fixed in the manifest"));
    }
    Ok(())
}

fn check_vendors(vendors: &[String]) -> Result<(), ManifestError> {
    if vendors.len() > MAX_VENDORS {
        return Err(bad("vendors", format!("at most {MAX_VENDORS}")));
    }
    for (i, v) in vendors.iter().enumerate() {
        let ok = !v.is_empty() && v.len() <= 32 && v.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
        if !ok {
            return Err(bad(&format!("vendors[{i}]"), "lowercase letters, digits and -, at most 32 characters"));
        }
    }
    Ok(())
}

fn check_install(install: &Install, command: &str) -> Result<(), ManifestError> {
    match install {
        Install::Path => Ok(()),
        Install::Npm { package, version } => {
            check_installed_command(command)?;
            check_npm_package(package)?;
            if !is_exact_version(version) {
                return Err(bad("install.version", "an exact version such as 1.2.3, never latest or a range"));
            }
            Ok(())
        }
        Install::Download { urls } => check_download(urls, command),
    }
}

/// A program AgentMux installs is named bare: AgentMux knows where it put it.
fn check_installed_command(command: &str) -> Result<(), ManifestError> {
    if std::path::Path::new(command).is_absolute() {
        return Err(bad("command", "an installed program is named bare (npm or download): AgentMux knows where it put it"));
    }
    Ok(())
}

fn check_npm_package(package: &str) -> Result<(), ManifestError> {
    let ok = !package.is_empty()
        && package.len() <= 214
        && !package.starts_with(['.', '_', '-'])
        && !package.contains("..")
        && package.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '@' | '/' | '.' | '_' | '-'))
        && package.matches('/').count() <= 1
        && package.matches('@').count() <= 1
        && (package.starts_with('@') || !package.contains('/'));
    if ok {
        Ok(())
    } else {
        Err(bad("install.package", "an npm package name such as tool or @scope/tool"))
    }
}

/// `1.2.3`, optionally `-pre.1` and `+build`: numbers only in the core.
fn is_exact_version(version: &str) -> bool {
    let (core, tail) = match version.find(['-', '+']) {
        Some(at) => (&version[..at], &version[at + 1..]),
        None => (version, "x"),
    };
    let parts: Vec<&str> = core.split('.').collect();
    let core_ok = parts.len() == 3 && parts.iter().all(|p| !p.is_empty() && p.len() <= 9 && p.chars().all(|c| c.is_ascii_digit()));
    let tail_ok = !tail.is_empty() && tail.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+'));
    core_ok && tail_ok
}

fn check_download(urls: &BTreeMap<String, DownloadAsset>, command: &str) -> Result<(), ManifestError> {
    if urls.is_empty() {
        return Err(bad("install.urls", "at least one platform"));
    }
    check_installed_command(command)?;
    for (platform, asset) in urls {
        let field = format!("install.urls.{platform}");
        if !is_platform_key(platform) {
            return Err(bad(&field, "a platform key: windows-x86_64, macos-aarch64, macos-x86_64, linux-x86_64 or linux-aarch64"));
        }
        if !asset.url.starts_with("https://") || asset.url.len() > 2048 || has_control(&asset.url) || asset.url.contains(' ') {
            return Err(bad(&format!("{field}.url"), "an https address"));
        }
        let hex_ok = asset.sha256.len() == 64 && asset.sha256.chars().all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c));
        if !hex_ok {
            return Err(bad(&format!("{field}.sha256"), "64 lowercase hex characters"));
        }
    }
    Ok(())
}

fn is_platform_key(key: &str) -> bool {
    matches!(key, "windows-x86_64" | "windows-aarch64" | "macos-aarch64" | "macos-x86_64" | "linux-x86_64" | "linux-aarch64")
}

#[cfg(test)]
#[path = "harness_manifest_tests.rs"]
mod tests;
