// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The install recipes of the agent CLIs that AgentMux can't install into its own folder: a
//! compiled table, so what runs on the user's machine comes from srv and never from a caller
//! (SPEC_ONE_CLICK_VENDOR_CLI_INSTALL_2026_10_20.md §4.1).
//!
//! The table holds each vendor's own installer, the same command the user would paste. Each is
//! a script fetched from the vendor's site at install time; the host list below pins
//! where that script may come from, redirects included, and nothing more. What the script then
//! downloads is the vendor's business, and the consent says so.
//!
//! A CLI AgentMux installs itself (`cli_download::SPECS`, a pinned release with a checked hash, or
//! `cli_uv::UV_SPECS`, a hash-locked Python venv) is not in this table: its Install button is the
//! managed one, and a test fails if a provider is in both. Every CLI AgentMux has is now installed
//! that way, so the table is empty; add a row only for a CLI it cannot install itself.

/// What runs a downloaded script.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Interpreter {
    Bash,
    Sh,
    PowerShell,
}

impl Interpreter {
    /// The program and the arguments before the script's path. Never a shell string.
    pub(crate) fn program_and_args(self) -> (&'static str, &'static [&'static str]) {
        match self {
            Self::Bash => ("bash", &[]),
            Self::Sh => ("sh", &[]),
            // Process-scoped: a downloaded `.ps1` is blocked by the default policy on many machines,
            // and the `irm | iex` the vendor documents is not subject to it. Nothing persists.
            Self::PowerShell => ("powershell", &["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File"]),
        }
    }

    pub(crate) fn extension(self) -> &'static str {
        match self {
            Self::PowerShell => "ps1",
            Self::Bash | Self::Sh => "sh",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Os {
    Windows,
    Unix,
}

impl Os {
    pub(crate) fn current() -> Self {
        if cfg!(windows) {
            Self::Windows
        } else {
            Self::Unix
        }
    }
}

#[derive(Debug)]
pub(crate) enum Recipe {
    /// Download `url`, show its hash, run that file with `interpreter`. `shown` is the vendor's
    /// documented one-liner, for Copy.
    Script {
        shown: &'static str,
        url: &'static str,
        interpreter: Interpreter,
        env: &'static [(&'static str, &'static str)],
    },
}

impl Recipe {
    /// The command the vendor documents: what Copy puts on the clipboard.
    pub(crate) fn shown(&self) -> String {
        match self {
            Self::Script { shown, .. } => (*shown).to_string(),
        }
    }

    /// What AgentMux runs, as the consent states it.
    pub(crate) fn runs(&self) -> String {
        match self {
            Self::Script { url, interpreter, .. } => {
                let (program, args) = interpreter.program_and_args();
                let args = if args.is_empty() { String::new() } else { format!(" {}", args.join(" ")) };
                format!("download {url}, then run that file with: {program}{args} <the downloaded file>")
            }
        }
    }

    pub(crate) fn url(&self) -> Option<&'static str> {
        match self {
            Self::Script { url, .. } => Some(url),
        }
    }
}

#[derive(Debug)]
pub(crate) struct VendorInstall {
    pub provider_id: &'static str,
    /// What the panel calls it.
    pub display: &'static str,
    pub unix: Recipe,
    pub windows: Recipe,
    /// Hosts the script may be fetched from, redirects included. An entry starting with `.`
    /// matches any subdomain of the rest (`.githubusercontent.com`); the others match exactly.
    pub allowed_hosts: &'static [&'static str],
    /// Longest the whole install may run.
    pub timeout_secs: u64,
}

impl VendorInstall {
    pub(crate) fn recipe(&self, os: Os) -> &Recipe {
        match os {
            Os::Windows => &self.windows,
            Os::Unix => &self.unix,
        }
    }
}

const TEN_MINUTES: u64 = 600;

pub(crate) const VENDOR_INSTALLS: &[VendorInstall] = &[];

/// Rows only the tests see, so the machinery that runs a vendor's installer stays covered
/// while the real table is empty.
#[cfg(test)]
pub(crate) const TEST_INSTALLS: &[VendorInstall] = &[
    VendorInstall {
        provider_id: "antigravity",
        display: "Antigravity",
        unix: Recipe::Script {
            shown: "curl -fsSL https://antigravity.google/cli/install.sh | bash",
            url: "https://antigravity.google/cli/install.sh",
            interpreter: Interpreter::Bash,
            env: &[],
        },
        windows: Recipe::Script {
            shown: "irm https://antigravity.google/cli/install.ps1 | iex",
            url: "https://antigravity.google/cli/install.ps1",
            interpreter: Interpreter::PowerShell,
            env: &[],
        },
        allowed_hosts: &["antigravity.google"],
        timeout_secs: TEN_MINUTES,
    },
];

/// Every row `vendor_install` can find: the table, and under test the fixture rows too.
pub(crate) fn all_installs() -> impl Iterator<Item = &'static VendorInstall> {
    #[cfg(test)]
    {
        VENDOR_INSTALLS.iter().chain(TEST_INSTALLS)
    }
    #[cfg(not(test))]
    {
        VENDOR_INSTALLS.iter()
    }
}

pub(crate) fn vendor_install(provider_id: &str) -> Option<&'static VendorInstall> {
    all_installs().find(|v| v.provider_id == provider_id)
}

/// Whether a script may be fetched from `url`: HTTPS on the default port, no credentials in
/// the address, and a host on the provider's list. Checked for the first request and for every
/// redirect.
pub(crate) fn url_allowed(allowed_hosts: &[&str], url: &reqwest::Url) -> bool {
    if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some() {
        return false;
    }
    if url.port().is_some_and(|p| p != 443) {
        return false;
    }
    let Some(host) = url.host_str() else { return false };
    let host = host.to_ascii_lowercase();
    allowed_hosts.iter().any(|allowed| match allowed.strip_prefix('.') {
        Some(_) => host.ends_with(allowed) && host.len() > allowed.len(),
        None => host == *allowed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(s: &str) -> reqwest::Url {
        reqwest::Url::parse(s).unwrap()
    }

    #[test]
    fn the_table_is_empty_because_agentmux_installs_every_cli_itself() {
        let ids: Vec<&str> = VENDOR_INSTALLS.iter().map(|v| v.provider_id).collect();
        assert!(ids.is_empty(), "{ids:?}: a CLI AgentMux can install itself belongs in cli_download::SPECS or cli_uv::UV_SPECS");
    }

    #[test]
    fn every_provider_in_the_table_is_a_registered_one_without_an_npm_package() {
        for v in VENDOR_INSTALLS {
            let p = crate::backend::providers::get_provider(v.provider_id).unwrap_or_else(|| panic!("{} is not a provider", v.provider_id));
            assert!(p.npm_package.is_empty(), "{} is installed by AgentMux itself", v.provider_id);
        }
    }

    /// A CLI AgentMux downloads itself has its own Install; offering the vendor's script too would
    /// show two ways to install one CLI. When a CLI gains a download spec, drop its row above.
    #[test]
    fn a_cli_agentmux_downloads_itself_is_not_in_the_vendor_table() {
        for v in VENDOR_INSTALLS {
            assert!(
                !crate::backend::cli_download::is_managed_install(v.provider_id),
                "{} is installed by AgentMux (cli_download::SPECS or cli_uv::UV_SPECS): remove it from VENDOR_INSTALLS",
                v.provider_id
            );
        }
    }

    #[test]
    fn every_script_url_is_https_and_passes_its_own_host_list() {
        for v in all_installs() {
            for os in [Os::Windows, Os::Unix] {
                match v.recipe(os) {
                    Recipe::Script { url: u, shown, .. } => {
                        assert!(url_allowed(v.allowed_hosts, &url(u)), "{} {os:?}: {u}", v.provider_id);
                        assert!(shown.contains(u), "{} {os:?}: the shown command names its own URL", v.provider_id);
                    }
                }
            }
        }
    }

    #[test]
    fn a_script_runs_by_the_interpreter_of_its_os() {
        for v in all_installs() {
            if let Recipe::Script { interpreter, url, .. } = v.recipe(Os::Windows) {
                assert_eq!(*interpreter, Interpreter::PowerShell, "{}", v.provider_id);
                assert!(url.ends_with(".ps1"), "{url}");
            }
            if let Recipe::Script { interpreter, url, .. } = v.recipe(Os::Unix) {
                assert_ne!(*interpreter, Interpreter::PowerShell, "{}", v.provider_id);
                assert!(url.ends_with(".sh"), "{url}");
            }
        }
    }

    #[test]
    fn no_interpreter_is_a_shell_string_or_a_pipe() {
        for i in [Interpreter::Bash, Interpreter::Sh, Interpreter::PowerShell] {
            let (program, args) = i.program_and_args();
            assert!(!program.contains(' ') && !program.contains('|'));
            assert!(args.iter().all(|a| !a.contains('|') && !a.contains(';')));
            assert!(!args.contains(&"-Command") && !args.contains(&"-c"));
        }
        let (_, ps) = Interpreter::PowerShell.program_and_args();
        assert_eq!(ps.last(), Some(&"-File"), "the downloaded file is what runs");
    }

    #[test]
    fn the_consent_names_what_runs() {
        let agy = vendor_install("antigravity").unwrap();
        let unix = agy.recipe(Os::Unix).runs();
        assert!(unix.contains("install.sh") && unix.contains("bash"), "{unix}");
    }

    #[test]
    fn an_http_url_is_refused() {
        assert!(!url_allowed(&["x.ai"], &url("http://x.ai/cli/install.sh")));
    }

    #[test]
    fn a_host_outside_the_list_is_refused_even_by_a_look_alike() {
        let hosts = &["x.ai", ".githubusercontent.com"];
        assert!(url_allowed(hosts, &url("https://x.ai/cli/install.sh")));
        assert!(!url_allowed(hosts, &url("https://evil.example/x.ai/cli/install.sh")));
        assert!(!url_allowed(hosts, &url("https://x.ai.evil.example/install.sh")));
        assert!(!url_allowed(hosts, &url("https://notx.ai/install.sh")));
        assert!(!url_allowed(hosts, &url("https://sub.x.ai/install.sh")), "an exact entry has no subdomains");
    }

    #[test]
    fn a_dotted_entry_matches_subdomains_only() {
        let hosts = &[".githubusercontent.com"];
        assert!(url_allowed(hosts, &url("https://release-assets.githubusercontent.com/a?b=c")));
        assert!(url_allowed(hosts, &url("https://RAW.GitHubUserContent.com/a")));
        assert!(!url_allowed(hosts, &url("https://githubusercontent.com/a")));
        assert!(!url_allowed(hosts, &url("https://evilgithubusercontent.com/a")));
    }

    #[test]
    fn credentials_and_odd_ports_in_the_address_are_refused() {
        let hosts = &["x.ai"];
        assert!(!url_allowed(hosts, &url("https://user:pw@x.ai/install.sh")));
        assert!(!url_allowed(hosts, &url("https://user@x.ai/install.sh")));
        assert!(!url_allowed(hosts, &url("https://x.ai:8443/install.sh")));
        assert!(url_allowed(hosts, &url("https://x.ai:443/install.sh")));
    }
}
