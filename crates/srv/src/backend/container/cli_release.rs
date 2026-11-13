// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The container route for the agent CLIs that are not on npm: goose, grok, muse and antigravity (a pinned release
//! file) and aider and kimi (a pinned `uv` and a hash-locked wheel set). The same registries as the host's own copy
//! (`backend::cli_download::SPECS`, `backend::cli_uv::UV_SPECS`) give the URL, the SHA-256 and the size, so one pin
//! covers both. No vendor script is run, and nothing a caller sends decides what is fetched: the script gets its
//! values from the registry as positional parameters, never spliced into the script text.
//!
//! The install lands in the per-agent volume (`/home/agent/.agentmux/cli`) beside the npm installs and ends with the
//! same completion marker: `bin/<command>` for a release file, `venv/bin/<command>` for a Python tool.
//! SPEC_ISOLATED_INSTALL_FOR_NON_NPM_CLIS_2026_11_05.md, PR 5d.

use std::time::Duration;

use crate::backend::cli_download::{self, Archive, Asset, DownloadSpec};
use crate::backend::cli_uv::{self, UvSpec};
use crate::backend::container::cli::InstallLimits;

/// How a CLI gets into the container.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// `npm install` of the provider's package (the usual route).
    Npm,
    /// One pinned file from the vendor, checked against a SHA-256 in the registry.
    Download,
    /// A pinned `uv` and a hash-locked set of wheels.
    Uv,
}

/// A file the install script reads, written into the container's `/tmp` before it runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Upload {
    pub name: String,
    pub bytes: Vec<u8>,
}

impl Source {
    /// The time limits of one install. A download is one file; a Python install fetches an interpreter and then
    /// the wheels (the host allows 20 minutes a step).
    pub fn limits(self) -> InstallLimits {
        match self {
            Source::Npm | Source::Download => InstallLimits::default(),
            Source::Uv => InstallLimits { run: Duration::from_secs(2700), script: Duration::from_secs(1200) },
        }
    }
}

/// The registry entry for a provider's non-npm route, if it has one that can run in a Linux container.
pub fn source_for(provider_id: &str) -> Option<(Source, &'static str)> {
    if let Some(spec) = cli_download::download_spec(provider_id) {
        return has_linux(spec.assets).then_some((Source::Download, spec.version));
    }
    let spec = cli_uv::uv_spec(provider_id)?;
    has_linux(cli_uv::UV.assets).then_some((Source::Uv, spec.version))
}

fn has_linux(assets: &[Asset]) -> bool {
    ["x86_64", "aarch64"].iter().all(|arch| assets.iter().any(|a| a.os == "linux" && a.arch == *arch))
}

fn kind_word(a: Archive) -> &'static str {
    match a {
        Archive::TarGz => "tar.gz",
        Archive::Raw => "raw",
        // No Linux asset is a zip; the script refuses the word, so one added later fails loudly.
        Archive::Zip => "zip",
    }
}

/// Six positional parameters per Linux asset: machine, URL, SHA-256, size, archive kind, executable path.
fn asset_params(assets: &[Asset]) -> Vec<String> {
    let mut out = Vec::new();
    for a in assets.iter().filter(|a| a.os == "linux") {
        out.extend([
            a.arch.to_string(),
            a.url.to_string(),
            a.sha256.to_string(),
            a.size.to_string(),
            kind_word(a.archive).to_string(),
            a.exe_in_archive.to_string(),
        ]);
    }
    out
}

fn head(script: &str, dir: &str, marker: &str, command: &str, limits: InstallLimits) -> Vec<String> {
    vec![
        "sh".into(),
        "-c".into(),
        script.into(),
        "agentmux-cli-install".into(),
        dir.into(),
        marker.into(),
        command.into(),
        limits.script.as_secs().max(1).to_string(),
    ]
}

/// argv that installs a pinned release file. `None` when the provider has no entry in the download registry.
pub fn download_argv(dir: &str, marker: &str, command: &str, provider_id: &str, limits: InstallLimits) -> Option<Vec<String>> {
    let spec: &DownloadSpec = cli_download::download_spec(provider_id)?;
    let mut argv = head(DOWNLOAD_SCRIPT.as_str(), dir, marker, command, limits);
    argv.extend(asset_params(spec.assets));
    Some(argv)
}

/// The names of the two files [`uv_argv`] reads from `/tmp`.
pub fn uv_upload_names(provider_id: &str) -> (String, String) {
    (format!("agentmux-lock-{provider_id}.txt"), format!("agentmux-tools-{provider_id}.txt"))
}

/// What the Python install reads: the hash-locked requirements and, for a package built from its sdist, the build
/// tools' entries of that lock.
pub fn uv_uploads(provider_id: &str) -> Vec<Upload> {
    let Some(spec) = cli_uv::uv_spec(provider_id) else { return Vec::new() };
    let (lock, tools) = uv_upload_names(provider_id);
    vec![
        Upload { name: lock, bytes: spec.lock.as_bytes().to_vec() },
        Upload { name: tools, bytes: cli_uv::build_tools_lock(spec).into_bytes() },
    ]
}

/// argv that installs a Python tool with the pinned `uv`. `None` when the provider has no entry in the uv registry.
pub fn uv_argv(dir: &str, marker: &str, command: &str, provider_id: &str, limits: InstallLimits) -> Option<Vec<String>> {
    let spec: &UvSpec = cli_uv::uv_spec(provider_id)?;
    let (lock, tools) = uv_upload_names(provider_id);
    let mut argv = head(UV_SCRIPT.as_str(), dir, marker, command, limits);
    argv.extend([spec.python.to_string(), format!("/tmp/{lock}"), format!("/tmp/{tools}"), spec.build_from_sdist.join(" ")]);
    argv.extend(asset_params(cli_uv::UV.assets));
    Some(argv)
}

/// The steps both scripts share. `$1` install dir, `$2` marker, `$3` command, `$4` seconds one network step may
/// run; the rest is the script's own.
const COMMON: &str = r#"set -eu
DIR="$1"; MARKER="$2"; BIN="$3"; LIMIT="$4"; shift 4
case "$(uname -m)" in
    x86_64|amd64) ARCH=x86_64 ;;
    aarch64|arm64) ARCH=aarch64 ;;
    *) echo "there is no build of $BIN for $(uname -m)" >&2; exit 3 ;;
esac
# pick MACHINE URL SHA SIZE KIND EXE ...: this machine's group of six, as URL SHA SIZE KIND EXE
pick() {
    URL=""
    while [ "$#" -ge 6 ]; do
        if [ "$1" = "$ARCH" ]; then URL="$2"; SHA="$3"; SIZE="$4"; KIND="$5"; EXE="$6"; fi
        shift 6
    done
    [ -n "$URL" ] || { echo "there is no build of $BIN for $ARCH Linux" >&2; exit 3; }
}
# fetch URL SHA SIZE OUT: https only, no more than the pinned size, and exactly the pinned hash
fetch() {
    MAX=$(( $3 + 1048576 ))
    timeout -k 15 "$LIMIT" curl -fsSL --proto '=https' --tlsv1.2 --connect-timeout 15 --max-filesize "$MAX" -o "$4" "$1"
    [ "$(wc -c < "$4")" -le "$MAX" ] || { echo "the download is larger than the pinned file" >&2; exit 4; }
    GOT=$(sha256sum "$4" | cut -d' ' -f1)
    [ "$GOT" = "$2" ] || { echo "the download does not match the pinned hash (expected $2, got $GOT)" >&2; exit 4; }
}
# unpack KIND FILE EXE OUT: the one executable, made 0755. An archive is unpacked into a throwaway folder; tar
# refuses a member that names a parent folder, and only a plain file is taken from it.
unpack() {
    case "$1" in
        raw) cp "$2" "$4" ;;
        tar.gz)
            X="$STAGE/unpacked"; mkdir -p "$X"
            tar -xzf "$2" -C "$X" --no-same-owner
            [ -f "$X/$3" ] && [ ! -L "$X/$3" ] || { echo "the archive has no file $3" >&2; exit 4; }
            cp "$X/$3" "$4" ;;
        *) echo "unknown archive kind $1" >&2; exit 4 ;;
    esac
    chmod 0755 "$4"
}
STAGE="$DIR/.staging"
mkdir -p "$DIR"
"#;

/// `$1...` six parameters per Linux asset of the CLI. The staging folder is inside the volume, so an attempt that
/// dies half-way never leaves a half-written `bin/`; the marker is written last.
const DOWNLOAD_BODY: &str = r#"pick "$@"
rm -rf "$STAGE"
mkdir -p "$STAGE/bin"
trap 'rm -rf "$STAGE"' EXIT
fetch "$URL" "$SHA" "$SIZE" "$STAGE/download"
unpack "$KIND" "$STAGE/download" "$EXE" "$STAGE/bin/$BIN"
rm -f "$DIR"/.installed-*
rm -rf "$DIR/bin"
mv "$STAGE/bin" "$DIR/bin"
: > "$DIR/$MARKER""#;

/// `$1` Python version, `$2` hash-locked requirements, `$3` the build tools' lock entries, `$4` packages built from
/// their sdist (space separated, may be empty), then six parameters per Linux asset of `uv`.
///
/// The venv is made where it will live (its scripts name its own path), so a failed attempt removes it, and an
/// older pin's marker goes first: "marker present" always means "complete".
const UV_BODY: &str = r#"PYTHON="$1"; LOCK="$2"; TOOLS="$3"; SDIST="$4"; shift 4
pick "$@"
rm -f "$DIR"/.installed-*
rm -rf "$STAGE" "$DIR/venv" "$DIR/python" "$DIR/python-bin" "$DIR/.uv-cache"
mkdir -p "$STAGE"
trap 'rc=$?; rm -rf "$STAGE" "$DIR/.uv-cache" "$LOCK" "$TOOLS"; [ -f "$DIR/$MARKER" ] || rm -rf "$DIR/venv" "$DIR/python" "$DIR/python-bin"; exit $rc' EXIT
fetch "$URL" "$SHA" "$SIZE" "$STAGE/uv.download"
unpack "$KIND" "$STAGE/uv.download" "$EXE" "$STAGE/uv"
export UV_CACHE_DIR="$DIR/.uv-cache" UV_PYTHON_INSTALL_DIR="$DIR/python" UV_PYTHON_BIN_DIR="$DIR/python-bin" \
    UV_NO_MODIFY_PATH=1 UV_PYTHON_DOWNLOADS=automatic UV_NO_PROGRESS=1 NO_COLOR=1
unset UV_INDEX_URL UV_DEFAULT_INDEX UV_EXTRA_INDEX_URL UV_INDEX UV_FIND_LINKS UV_PYTHON UV_PROJECT_ENVIRONMENT \
    UV_REQUIRE_HASHES VIRTUAL_ENV PIP_INDEX_URL PIP_EXTRA_INDEX_URL PIP_FIND_LINKS
UV="$STAGE/uv"
timeout -k 15 "$LIMIT" "$UV" venv --managed-python --python "$PYTHON" --no-config "$DIR/venv"
set -- pip install --no-config --python "$DIR/venv/bin/python" --default-index https://pypi.org/simple \
    --require-hashes --only-binary :all: --no-deps
if [ -n "$SDIST" ]; then
    timeout -k 15 "$LIMIT" "$UV" "$@" -r "$TOOLS"
    for p in $SDIST; do set -- "$@" --no-binary "$p"; done
    set -- "$@" --no-build-isolation
fi
timeout -k 15 "$LIMIT" "$UV" "$@" -r "$LOCK"
"$DIR/venv/bin/$BIN" --version >/dev/null
: > "$DIR/$MARKER""#;

/// The two scripts, assembled once.
pub static DOWNLOAD_SCRIPT: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| format!("{COMMON}{DOWNLOAD_BODY}"));
pub static UV_SCRIPT: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| format!("{COMMON}{UV_BODY}"));

#[cfg(test)]
#[path = "cli_release_tests.rs"]
mod tests;
