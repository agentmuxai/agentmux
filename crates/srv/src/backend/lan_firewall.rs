// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Would the OS firewall let LAN peers reach us? Pure rule logic, no OS calls.
//!
//! SPEC_LAN_FIREWALL_SETUP_2026_10_01.md 4.2 and 4.3. On 2026-09-30 narko had LAN
//! enabled, started its listeners, registered its mDNS service, logged no
//! warning, and never saw a peer, because Windows Defender Firewall had no
//! inbound allow rule for the active Private profile (and 606 stale per-path
//! rules, none of them for the running binary). Nothing in AgentMux could say so.
//!
//! This module decides, from a snapshot of the firewall rules and the adapters'
//! network categories, whether each interface is covered, and what the status
//! bar should say. Reading the snapshot is OS-specific and lives elsewhere
//! (`lan_firewall_windows.rs`); keeping the decision pure means the cases that
//! actually bit us are unit tests that run on every platform.
//!
//! Windows semantics encoded here:
//! - Inbound traffic is blocked unless a matching enabled **allow** rule exists,
//!   and an enabled matching **block** rule beats any allow.
//! - A rule applies per network **profile** (Domain, Private, Public), and each
//!   adapter's connection has its own category.
//! - A rule naming a program matches that exact path. The sidecar's file name
//!   embeds its version (`agentmux-srv-0.59.1-windows.x64.exe`) and portable and
//!   dev builds sit at a new path every time, so a per-program rule stops
//!   matching on every update (spec R7). A **port** rule does not.

use std::net::Ipv4Addr;

/// `NET_FW_PROFILE2_*` bits.
pub const PROFILE_DOMAIN: u32 = 1;
pub const PROFILE_PRIVATE: u32 = 2;
pub const PROFILE_PUBLIC: u32 = 4;

/// A network's category, as `INetworkListManager` reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Public,
    Private,
    Domain,
}

impl Category {
    pub fn bit(self) -> u32 {
        match self {
            Category::Domain => PROFILE_DOMAIN,
            Category::Private => PROFILE_PRIVATE,
            Category::Public => PROFILE_PUBLIC,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Proto {
    Tcp,
    Udp,
    Any,
    /// ICMP and anything else: never covers a TCP or UDP need.
    Other,
}

impl Proto {
    /// `NET_FW_IP_PROTOCOL_*`: TCP 6, UDP 17, ANY 256.
    pub fn from_win(n: i32) -> Proto {
        match n {
            6 => Proto::Tcp,
            17 => Proto::Udp,
            256 => Proto::Any,
            _ => Proto::Other,
        }
    }

    fn covers(self, want: Proto) -> bool {
        match self {
            Proto::Any => matches!(want, Proto::Tcp | Proto::Udp),
            Proto::Other => false,
            p => p == want,
        }
    }
}

/// A rule's `LocalPorts`. Anything we cannot read exactly is `Unparseable` and
/// matches nothing: claiming coverage we cannot prove is the worse error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PortSpec {
    Any,
    Ranges(Vec<(u16, u16)>),
    Unparseable,
}

impl PortSpec {
    /// `""` and `"*"` are any port; otherwise a comma list of ports and
    /// `low-high` ranges. Windows also allows keywords (`RPC`, `IPHTTPS`,
    /// `Teredo`, ...): those are `Unparseable`.
    pub fn parse(spec: &str) -> PortSpec {
        let spec = spec.trim();
        if spec.is_empty() || spec == "*" {
            return PortSpec::Any;
        }
        let mut ranges = Vec::new();
        for part in spec.split(',') {
            let part = part.trim();
            let (lo, hi) = match part.split_once('-') {
                Some((a, b)) => (a.trim().parse::<u16>(), b.trim().parse::<u16>()),
                None => {
                    let p = part.parse::<u16>();
                    (p.clone(), p)
                }
            };
            match (lo, hi) {
                (Ok(lo), Ok(hi)) if lo <= hi => ranges.push((lo, hi)),
                _ => return PortSpec::Unparseable,
            }
        }
        PortSpec::Ranges(ranges)
    }

    pub fn contains(&self, port: u16) -> bool {
        match self {
            PortSpec::Any => true,
            PortSpec::Ranges(r) => r.iter().any(|&(lo, hi)| (lo..=hi).contains(&port)),
            PortSpec::Unparseable => false,
        }
    }
}

/// A rule's `RemoteAddresses`, reduced to the one question that matters here:
/// does it reach LAN peers in general?
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Remote {
    /// `*` or empty.
    Any,
    /// `LocalSubnet`: the peers' subnet. Covers a LAN peer.
    LocalSubnet,
    /// A specific list, `Internet`, `DNS`, ...: does not cover peers in general.
    Other,
}

impl Remote {
    pub fn parse(spec: &str) -> Remote {
        let spec = spec.trim();
        if spec.is_empty() || spec == "*" {
            Remote::Any
        } else if spec.eq_ignore_ascii_case("LocalSubnet") {
            Remote::LocalSubnet
        } else {
            Remote::Other
        }
    }

    fn reaches_lan_peers(self) -> bool {
        matches!(self, Remote::Any | Remote::LocalSubnet)
    }
}

/// One firewall rule, with the fields that decide whether it affects us.
#[derive(Debug, Clone)]
pub struct FwRule {
    pub name: String,
    pub enabled: bool,
    pub inbound: bool,
    pub allow: bool,
    /// `NET_FW_PROFILE2_*` bitmask; Windows uses `0x7FFFFFFF` for "all".
    pub profiles: u32,
    /// The program the rule is bound to, `None` for any program.
    pub program: Option<String>,
    pub proto: Proto,
    pub local_ports: PortSpec,
    pub remote: Remote,
    /// Restricted to particular adapters or interface types. Such a rule may not
    /// cover the adapter we care about, so it is skipped rather than assumed.
    pub interface_scoped: bool,
    /// Bound to something other than a program path: a Windows service, a Store
    /// app package (`LocalAppPackageId`), particular users or machines, or secure
    /// (authenticated) traffic only. Such a rule has an empty program path and
    /// looks like "any program", which it is not. Found on narko, 2026-10-01: 25
    /// Store-app rules ("Solitaire", "Microsoft Store", ...) made coverage look
    /// complete for a binary no rule named. Skipped, never assumed to apply.
    pub restricted: bool,
}

/// Compare two Windows paths: case-insensitive, `/` equals `\`, the `\\?\`
/// prefix ignored. Rules that use environment variables (`%SystemRoot%`) do not
/// match: coverage is only claimed when it can be shown.
pub fn same_path(a: &str, b: &str) -> bool {
    fn norm(p: &str) -> String {
        p.trim()
            .trim_start_matches(r"\\?\")
            .replace('/', "\\")
            .to_lowercase()
    }
    let (a, b) = (norm(a), norm(b));
    !a.is_empty() && a == b
}

/// Something the running srv must be reachable on from the LAN.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Need {
    pub proto: Proto,
    pub port: u16,
}

/// The inbound traffic a LAN peer needs: our web and ws ports (TCP) and the
/// mDNS port (UDP 5353).
pub fn lan_needs(web_port: u16, ws_port: u16) -> Vec<Need> {
    vec![
        Need { proto: Proto::Tcp, port: web_port },
        Need { proto: Proto::Tcp, port: ws_port },
        Need { proto: Proto::Udp, port: 5353 },
    ]
}

fn rule_applies(rule: &FwRule, exe: &str, need: Need, category: Category) -> bool {
    rule.enabled
        && rule.inbound
        && rule.profiles & category.bit() != 0
        && !rule.interface_scoped
        && !rule.restricted
        && rule.proto.covers(need.proto)
        && rule.local_ports.contains(need.port)
        && rule.remote.reaches_lan_peers()
        && rule.program.as_deref().is_none_or(|p| same_path(p, exe))
}

/// The rules that decide `need` for `category`, allow or block. For diagnostics
/// and tests: it is the same predicate `coverage` uses.
pub fn matching_rules<'a>(
    rules: &'a [FwRule],
    exe: &str,
    need: Need,
    category: Category,
    allow: bool,
) -> Vec<&'a FwRule> {
    rules
        .iter()
        .filter(|r| r.allow == allow && rule_applies(r, exe, need, category))
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Coverage {
    /// Every need is allowed and none is blocked.
    Covered,
    /// An enabled inbound block rule matches at least one need.
    Blocked,
    /// No block, but at least one need has no allow rule.
    Missing,
}

/// Does `category`'s profile let LAN peers reach `exe` on every need?
/// A matching block rule wins over any allow, as it does in Windows.
pub fn coverage(rules: &[FwRule], exe: &str, needs: &[Need], category: Category) -> Coverage {
    let applies = |allow: bool, need: Need| {
        rules
            .iter()
            .any(|r| r.allow == allow && rule_applies(r, exe, need, category))
    };
    if needs.iter().any(|&n| applies(false, n)) {
        return Coverage::Blocked;
    }
    if needs.iter().all(|&n| applies(true, n)) {
        Coverage::Covered
    } else {
        Coverage::Missing
    }
}

/// A network adapter and the category of the network it is connected to.
#[derive(Debug, Clone)]
pub struct Adapter {
    /// The adapter GUID (`if_addrs::Interface::adapter_name` on Windows).
    pub id: String,
    pub name: String,
    pub ipv4: Vec<Ipv4Addr>,
    /// `None` when Windows could not say (no profile yet, or the lookup failed).
    pub category: Option<Category>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdapterState {
    Covered,
    NeedsSetup,
    Blocked,
    /// A Public network with no allow rule: not set up on purpose. Opening it
    /// needs the user's consent for that network (spec 4.4), unlike Private.
    PublicNotTrusted,
    /// Category unknown: no claim either way.
    Unknown,
}

pub fn adapter_state(rules: &[FwRule], exe: &str, needs: &[Need], adapter: &Adapter) -> AdapterState {
    let Some(category) = adapter.category else {
        return AdapterState::Unknown;
    };
    match coverage(rules, exe, needs, category) {
        Coverage::Covered => AdapterState::Covered,
        Coverage::Blocked => AdapterState::Blocked,
        Coverage::Missing if category == Category::Public => AdapterState::PublicNotTrusted,
        Coverage::Missing => AdapterState::NeedsSetup,
    }
}

/// What the status bar shows about the firewall (spec 4.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FirewallStatus {
    /// At least one adapter is covered: LAN can work.
    Ok,
    NeedsSetup,
    Blocked,
    PublicNetwork,
    /// An administrator's policy makes local rules ineffective: a setup helper
    /// cannot fix it.
    Managed,
    /// Nothing can be said (no adapters, or categories unknown).
    Unknown,
}

impl FirewallStatus {
    pub fn wire(self) -> &'static str {
        match self {
            FirewallStatus::Ok => "ok",
            FirewallStatus::NeedsSetup => "needs-setup",
            FirewallStatus::Blocked => "blocked",
            FirewallStatus::PublicNetwork => "public-network",
            FirewallStatus::Managed => "managed",
            FirewallStatus::Unknown => "unknown",
        }
    }
}

/// The overall status from every adapter. One covered adapter is enough for
/// "ok": a host also has virtual adapters (VMware, WSL, Hyper-V) that are
/// rightly uncovered, and they must not flag a machine whose real LAN works.
/// Otherwise the most actionable problem wins: a block, then a managed policy
/// (no helper can help), then missing rules, then an untrusted Public network.
pub fn overall(states: &[AdapterState], local_rules_ignored: bool) -> FirewallStatus {
    let any = |s: AdapterState| states.iter().any(|&x| x == s);
    if any(AdapterState::Covered) {
        FirewallStatus::Ok
    } else if any(AdapterState::Blocked) {
        FirewallStatus::Blocked
    } else if local_rules_ignored && (any(AdapterState::NeedsSetup) || any(AdapterState::PublicNotTrusted)) {
        FirewallStatus::Managed
    } else if any(AdapterState::NeedsSetup) {
        FirewallStatus::NeedsSetup
    } else if any(AdapterState::PublicNotTrusted) {
        FirewallStatus::PublicNetwork
    } else {
        FirewallStatus::Unknown
    }
}

/// Everything read from the OS in one go.
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub rules: Vec<FwRule>,
    pub adapters: Vec<Adapter>,
    /// Group policy stops locally created rules from applying.
    pub local_rules_ignored: bool,
}

#[derive(Debug, Clone)]
pub struct Report {
    pub status: FirewallStatus,
    pub per_adapter: Vec<(Adapter, AdapterState)>,
}

pub fn report(snapshot: &Snapshot, exe: &str, needs: &[Need]) -> Report {
    let per_adapter: Vec<(Adapter, AdapterState)> = snapshot
        .adapters
        .iter()
        .map(|a| (a.clone(), adapter_state(&snapshot.rules, exe, needs, a)))
        .collect();
    let states: Vec<AdapterState> = per_adapter.iter().map(|(_, s)| *s).collect();
    Report {
        status: overall(&states, snapshot.local_rules_ignored),
        per_adapter,
    }
}

/// Whether this status goes to the windows on this tick. A standing problem is
/// re-sent every tick so a window opened later, or a reconnected WebSocket,
/// learns of it: the frontend only learns from events and cannot ask (the same
/// lesson as `lan_mdns_health::should_publish`, ReAgent P1 on #4148). `Ok` and
/// `Unknown` go out only on change.
pub fn should_publish(status: FirewallStatus, changed: bool) -> bool {
    match status {
        FirewallStatus::Ok | FirewallStatus::Unknown => changed,
        FirewallStatus::NeedsSetup
        | FirewallStatus::Blocked
        | FirewallStatus::PublicNetwork
        | FirewallStatus::Managed => true,
    }
}

fn category_wire(c: Option<Category>) -> &'static str {
    match c {
        Some(Category::Public) => "public",
        Some(Category::Private) => "private",
        Some(Category::Domain) => "domain",
        None => "unknown",
    }
}

fn state_wire(s: AdapterState) -> &'static str {
    match s {
        AdapterState::Covered => "covered",
        AdapterState::NeedsSetup => "needs-setup",
        AdapterState::Blocked => "blocked",
        AdapterState::PublicNotTrusted => "public-not-trusted",
        AdapterState::Unknown => "unknown",
    }
}

/// The `laninstances:firewall` event body.
pub fn payload(
    status: FirewallStatus,
    per_adapter: &[(Adapter, AdapterState)],
    local_rules_ignored: bool,
) -> serde_json::Value {
    serde_json::json!({
        "status": status.wire(),
        "localRulesIgnored": local_rules_ignored,
        "adapters": per_adapter.iter().map(|(a, s)| serde_json::json!({
            "name": a.name,
            "category": category_wire(a.category),
            "state": state_wire(*s),
        })).collect::<Vec<_>>(),
    })
}

/// How often the firewall is read while LAN discovery is wanted. A read is cheap
/// (about 60 ms for 1,300 rules on narko) and a changed rule or network should
/// show promptly.
const WATCH_TICK: std::time::Duration = std::time::Duration::from_secs(30);

/// Watch the OS firewall while LAN discovery is wanted and tell the windows what
/// it found, via `laninstances:firewall`. Only Windows can read its firewall
/// today (macOS and Linux are later PRs): elsewhere this does nothing, so the
/// status bar simply has no firewall verdict.
///
/// Read-only. It changes no rule and gates nothing: the indicator tells the
/// truth first (spec 4.3); acting on it is a later step.
pub fn spawn_watcher(
    lan: std::sync::Arc<super::lan_discovery::LanDiscoveryController>,
    web_port: u16,
    ws_port: u16,
    event_bus: std::sync::Arc<super::eventbus::EventBus>,
) {
    if !cfg!(windows) {
        return;
    }
    tokio::spawn(async move {
        let mut last: Option<String> = None;
        let mut tick = tokio::time::interval(WATCH_TICK);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tick.tick().await;
            if !lan.is_wanted() {
                // LAN went off: clear whatever the indicator was showing.
                if last.as_deref().is_some_and(|s| s != "off") {
                    event_bus.broadcast_event(&super::eventbus::WSEventType {
                        eventtype: "laninstances:firewall".to_string(),
                        oref: String::new(),
                        data: Some(serde_json::json!({
                            "status": "off",
                            "adapters": [],
                            "localRulesIgnored": false
                        })),
                    });
                    last = Some("off".to_string());
                }
                continue;
            }
            let exe = match std::env::current_exe() {
                Ok(p) => p.to_string_lossy().to_string(),
                Err(e) => {
                    tracing::warn!("firewall check: cannot find our own executable: {e}");
                    continue;
                }
            };
            let snapshot = match tokio::task::spawn_blocking(read_snapshot).await {
                Ok(Ok(s)) => s,
                Ok(Err(e)) => {
                    // "Unknown" is a fact worth one line, not one per tick.
                    if last.as_deref() != Some("unknown") {
                        tracing::warn!("could not read the OS firewall, so no verdict: {e}");
                        last = Some("unknown".to_string());
                    }
                    continue;
                }
                Err(e) => {
                    tracing::warn!("firewall reader task failed: {e}");
                    continue;
                }
            };
            let report = report(&snapshot, &exe, &lan_needs(web_port, ws_port));
            let wire = report.status.wire().to_string();
            let changed = last.as_deref() != Some(wire.as_str());
            if changed && report.status != FirewallStatus::Ok {
                tracing::warn!(
                    status = %wire,
                    adapters = ?report.per_adapter.iter().map(|(a, s)| (a.name.clone(), *s)).collect::<Vec<_>>(),
                    "OS firewall may keep other machines from reaching this one"
                );
            }
            if should_publish(report.status, changed) {
                event_bus.broadcast_event(&super::eventbus::WSEventType {
                    eventtype: "laninstances:firewall".to_string(),
                    oref: String::new(),
                    data: Some(payload(report.status, &report.per_adapter, snapshot.local_rules_ignored)),
                });
            }
            last = Some(wire);
        }
    });
}

/// Read the OS firewall and the adapters' network categories. Windows only for
/// now (macOS is PR C, Linux PR D of the spec's delivery plan); elsewhere it is an
/// `Err`, which callers must treat as "unknown", never as "covered".
#[cfg(windows)]
pub use super::lan_firewall_windows::read_snapshot;

#[cfg(not(windows))]
pub fn read_snapshot() -> Result<Snapshot, String> {
    Err("firewall inspection is only implemented on Windows".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: u32 = 0x7FFF_FFFF;
    const EXE: &str = r"C:\Users\asafe\Desktop\agentmux-0.59.1\runtime\agentmux-srv-0.59.1-windows.x64.exe";
    const OLD_EXE: &str = r"C:\Users\asafe\Desktop\agentmux-0.58.2\runtime\agentmux-srv-0.58.2-windows.x64.exe";

    fn needs() -> Vec<Need> {
        lan_needs(29700, 29701)
    }

    fn rule(name: &str) -> FwRule {
        FwRule {
            name: name.to_string(),
            enabled: true,
            inbound: true,
            allow: true,
            profiles: PROFILE_PRIVATE,
            program: None,
            proto: Proto::Any,
            local_ports: PortSpec::Any,
            remote: Remote::Any,
            interface_scoped: false,
            restricted: false,
        }
    }

    fn program_rule(exe: &str, proto: Proto, profiles: u32, allow: bool) -> FwRule {
        FwRule {
            program: Some(exe.to_string()),
            proto,
            profiles,
            allow,
            ..rule("agentmux-srv")
        }
    }

    fn adapter(name: &str, cat: Option<Category>) -> Adapter {
        Adapter {
            id: format!("{{{name}}}"),
            name: name.to_string(),
            ipv4: vec!["192.168.1.26".parse().unwrap()],
            category: cat,
        }
    }

    // ---- parsers

    #[test]
    fn port_specs_parse_the_forms_windows_uses() {
        assert_eq!(PortSpec::parse(""), PortSpec::Any);
        assert_eq!(PortSpec::parse("*"), PortSpec::Any);
        let p = PortSpec::parse("5353, 29700-29799");
        assert!(p.contains(5353) && p.contains(29700) && p.contains(29799) && p.contains(29750));
        assert!(!p.contains(29800) && !p.contains(5354) && !p.contains(29699));
    }

    #[test]
    fn port_keywords_and_junk_never_match() {
        for bad in ["RPC", "IPHTTPS", "Teredo", "80-", "-80", "70000", "90-80", "1,,2", "a-b"] {
            let p = PortSpec::parse(bad);
            assert_eq!(p, PortSpec::Unparseable, "{bad}");
            assert!(!p.contains(80));
        }
    }

    #[test]
    fn remote_addresses_reduce_to_whether_lan_peers_are_reached() {
        assert!(Remote::parse("").reaches_lan_peers());
        assert!(Remote::parse("*").reaches_lan_peers());
        assert!(Remote::parse("LocalSubnet").reaches_lan_peers());
        assert!(Remote::parse("localsubnet").reaches_lan_peers());
        assert!(!Remote::parse("192.168.1.5").reaches_lan_peers());
        assert!(!Remote::parse("Internet").reaches_lan_peers());
        assert!(!Remote::parse("LocalSubnet,10.0.0.1").reaches_lan_peers());
    }

    #[test]
    fn protocols_from_the_windows_numbers() {
        assert_eq!(Proto::from_win(6), Proto::Tcp);
        assert_eq!(Proto::from_win(17), Proto::Udp);
        assert_eq!(Proto::from_win(256), Proto::Any);
        assert_eq!(Proto::from_win(1), Proto::Other);
        assert!(!Proto::Other.covers(Proto::Tcp));
        assert!(Proto::Any.covers(Proto::Udp));
    }

    #[test]
    fn paths_compare_the_way_windows_does() {
        assert!(same_path(r"C:\A\B.exe", r"c:\a\b.EXE"));
        assert!(same_path("C:/A/B.exe", r"C:\A\B.exe"));
        assert!(same_path(r"\\?\C:\A\B.exe", r"C:\A\B.exe"));
        assert!(!same_path(r"C:\A\B.exe", r"C:\A\C.exe"));
        assert!(!same_path("", ""));
        assert!(!same_path(r"%SystemRoot%\x.exe", r"C:\Windows\x.exe"), "env vars are not expanded");
    }

    // ---- coverage: the cases that actually happened

    /// narko, 2026-09-30: 606 per-program rules, none for the running binary,
    /// nothing for the Private profile. LAN was on, nothing warned, no peer
    /// was ever seen.
    #[test]
    fn narko_with_only_stale_per_path_rules_needs_setup() {
        let mut rules: Vec<FwRule> = (0..606)
            .map(|i| program_rule(&format!(r"C:\old\{i}\agentmux-srv.exe"), Proto::Any, ALL, true))
            .collect();
        rules.push(program_rule(OLD_EXE, Proto::Any, ALL, true));
        assert_eq!(coverage(&rules, EXE, &needs(), Category::Private), Coverage::Missing);
        let a = adapter("Ethernet", Some(Category::Private));
        assert_eq!(adapter_state(&rules, EXE, &needs(), &a), AdapterState::NeedsSetup);
    }

    /// Area54: Private allow rules for the CURRENT binary, among 400 stale ones.
    #[test]
    fn area54_with_rules_for_the_current_binary_is_covered() {
        let mut rules: Vec<FwRule> = (0..390)
            .map(|i| program_rule(&format!(r"C:\old\{i}\agentmux-srv.exe"), Proto::Any, ALL, true))
            .collect();
        rules.push(program_rule(EXE, Proto::Tcp, PROFILE_PRIVATE, true));
        rules.push(program_rule(EXE, Proto::Udp, PROFILE_PRIVATE, true));
        // The Public blocks that Windows adds when a prompt is cancelled.
        rules.push(program_rule(EXE, Proto::Tcp, PROFILE_PUBLIC, false));
        rules.push(program_rule(EXE, Proto::Udp, PROFILE_PUBLIC, false));
        assert_eq!(coverage(&rules, EXE, &needs(), Category::Private), Coverage::Covered);
        assert_eq!(coverage(&rules, EXE, &needs(), Category::Public), Coverage::Blocked);
    }

    #[test]
    fn a_program_rule_for_another_path_does_not_cover_this_binary() {
        // Spec R7: the version is in the file name, so every update changes the path.
        let rules = vec![program_rule(OLD_EXE, Proto::Any, ALL, true)];
        assert_eq!(coverage(&rules, EXE, &needs(), Category::Private), Coverage::Missing);
    }

    #[test]
    fn port_rules_survive_updates_and_new_paths() {
        let tcp = FwRule {
            proto: Proto::Tcp,
            local_ports: PortSpec::parse("29700-29799"),
            remote: Remote::LocalSubnet,
            ..rule("AgentMux LAN (TCP)")
        };
        let udp = FwRule {
            proto: Proto::Udp,
            local_ports: PortSpec::parse("5353,47891"),
            remote: Remote::LocalSubnet,
            ..rule("AgentMux LAN (UDP)")
        };
        let rules = vec![tcp, udp];
        for exe in [EXE, OLD_EXE, r"D:\portable\anything.exe"] {
            assert_eq!(coverage(&rules, exe, &needs(), Category::Private), Coverage::Covered, "{exe}");
        }
    }

    #[test]
    fn srv_ports_outside_the_rule_range_are_not_covered() {
        // The OS-chosen fallback port when the fixed range is exhausted.
        let tcp = FwRule { proto: Proto::Tcp, local_ports: PortSpec::parse("29700-29799"), ..rule("t") };
        let udp = FwRule { proto: Proto::Udp, local_ports: PortSpec::parse("5353"), ..rule("u") };
        let outside = lan_needs(51234, 51235);
        assert_eq!(coverage(&[tcp, udp], EXE, &outside, Category::Private), Coverage::Missing);
    }

    // ---- what must not count

    #[test]
    fn disabled_outbound_interface_scoped_and_remote_limited_rules_do_not_count() {
        let base = program_rule(EXE, Proto::Any, PROFILE_PRIVATE, true);
        for broken in [
            FwRule { enabled: false, ..base.clone() },
            FwRule { inbound: false, ..base.clone() },
            FwRule { interface_scoped: true, ..base.clone() },
            // A Store-app or service rule looks like "any program" but is not.
            FwRule { restricted: true, program: None, ..base.clone() },
            FwRule { remote: Remote::Other, ..base.clone() },
            FwRule { proto: Proto::Other, ..base.clone() },
            FwRule { local_ports: PortSpec::Unparseable, ..base.clone() },
        ] {
            assert_eq!(coverage(&[broken.clone()], EXE, &needs(), Category::Private), Coverage::Missing, "{broken:?}");
        }
    }

    #[test]
    fn a_rule_for_another_profile_does_not_cover_this_one() {
        let rules = vec![program_rule(EXE, Proto::Any, PROFILE_PUBLIC | PROFILE_DOMAIN, true)];
        assert_eq!(coverage(&rules, EXE, &needs(), Category::Private), Coverage::Missing);
        assert_eq!(coverage(&rules, EXE, &needs(), Category::Domain), Coverage::Covered);
    }

    #[test]
    fn a_block_beats_an_allow_as_in_windows() {
        let rules = vec![
            program_rule(EXE, Proto::Any, ALL, true),
            program_rule(EXE, Proto::Tcp, PROFILE_PRIVATE, false),
        ];
        assert_eq!(coverage(&rules, EXE, &needs(), Category::Private), Coverage::Blocked);
        // The other profiles are untouched by it.
        assert_eq!(coverage(&rules, EXE, &needs(), Category::Domain), Coverage::Covered);
    }

    #[test]
    fn a_block_for_a_different_program_does_not_block_us() {
        let rules = vec![
            program_rule(EXE, Proto::Any, ALL, true),
            program_rule(OLD_EXE, Proto::Any, ALL, false),
        ];
        assert_eq!(coverage(&rules, EXE, &needs(), Category::Private), Coverage::Covered);
    }

    #[test]
    fn tcp_alone_is_not_enough_mdns_needs_udp_too() {
        let rules = vec![program_rule(EXE, Proto::Tcp, PROFILE_PRIVATE, true)];
        assert_eq!(coverage(&rules, EXE, &needs(), Category::Private), Coverage::Missing);
    }

    // ---- per adapter and overall

    #[test]
    fn a_public_adapter_with_no_rule_is_not_trusted_rather_than_needs_setup() {
        let rules = vec![program_rule(EXE, Proto::Any, PROFILE_PRIVATE, true)];
        let public = adapter("Wi-Fi", Some(Category::Public));
        assert_eq!(adapter_state(&rules, EXE, &needs(), &public), AdapterState::PublicNotTrusted);
        let private = adapter("Ethernet", Some(Category::Private));
        assert_eq!(adapter_state(&rules, EXE, &needs(), &private), AdapterState::Covered);
    }

    #[test]
    fn an_adapter_with_an_unknown_category_makes_no_claim() {
        let rules = vec![program_rule(EXE, Proto::Any, ALL, true)];
        assert_eq!(adapter_state(&rules, EXE, &needs(), &adapter("x", None)), AdapterState::Unknown);
    }

    #[test]
    fn one_covered_adapter_is_enough_virtual_adapters_do_not_flag_a_working_lan() {
        use AdapterState::*;
        assert_eq!(overall(&[Covered, PublicNotTrusted, NeedsSetup, Blocked], false), FirewallStatus::Ok);
    }

    #[test]
    fn overall_priority_is_block_then_managed_then_setup_then_public() {
        use AdapterState::*;
        assert_eq!(overall(&[NeedsSetup, Blocked], false), FirewallStatus::Blocked);
        assert_eq!(overall(&[NeedsSetup], true), FirewallStatus::Managed);
        assert_eq!(overall(&[PublicNotTrusted], true), FirewallStatus::Managed);
        assert_eq!(overall(&[PublicNotTrusted, NeedsSetup], false), FirewallStatus::NeedsSetup);
        assert_eq!(overall(&[PublicNotTrusted], false), FirewallStatus::PublicNetwork);
        assert_eq!(overall(&[Unknown, Unknown], false), FirewallStatus::Unknown);
        assert_eq!(overall(&[], false), FirewallStatus::Unknown);
    }

    #[test]
    fn a_managed_policy_does_not_hide_a_working_lan() {
        assert_eq!(overall(&[AdapterState::Covered], true), FirewallStatus::Ok);
    }

    #[test]
    fn the_report_ties_the_snapshot_together() {
        let snap = Snapshot {
            rules: vec![program_rule(EXE, Proto::Any, PROFILE_PRIVATE, true)],
            adapters: vec![
                adapter("Ethernet", Some(Category::Private)),
                adapter("vEthernet (Default Switch)", Some(Category::Public)),
            ],
            local_rules_ignored: false,
        };
        let r = report(&snap, EXE, &needs());
        assert_eq!(r.status, FirewallStatus::Ok);
        assert_eq!(r.per_adapter[0].1, AdapterState::Covered);
        assert_eq!(r.per_adapter[1].1, AdapterState::PublicNotTrusted);

        let none = Snapshot { rules: vec![], ..snap };
        assert_eq!(report(&none, EXE, &needs()).status, FirewallStatus::NeedsSetup);
    }

    #[test]
    fn a_standing_problem_is_resent_every_tick_ok_and_unknown_only_on_change() {
        for problem in [
            FirewallStatus::NeedsSetup,
            FirewallStatus::Blocked,
            FirewallStatus::PublicNetwork,
            FirewallStatus::Managed,
        ] {
            assert!(should_publish(problem, false), "{problem:?}: a late window must learn of it");
        }
        for quiet in [FirewallStatus::Ok, FirewallStatus::Unknown] {
            assert!(!should_publish(quiet, false));
            assert!(should_publish(quiet, true), "{quiet:?}: a change must clear an earlier warning");
        }
    }

    #[test]
    fn the_event_payload_is_the_frontend_contract() {
        let a = adapter("Ethernet", Some(Category::Private));
        let v = payload(FirewallStatus::NeedsSetup, &[(a, AdapterState::NeedsSetup)], false);
        assert_eq!(v["status"], "needs-setup");
        assert_eq!(v["localRulesIgnored"], false);
        assert_eq!(v["adapters"][0]["name"], "Ethernet");
        assert_eq!(v["adapters"][0]["category"], "private");
        assert_eq!(v["adapters"][0]["state"], "needs-setup");
    }

    #[test]
    fn a_rule_bound_to_a_service_or_store_app_is_not_an_any_program_rule() {
        // Found on narko: 25 Store-app rules have an empty program path.
        let store = FwRule { restricted: true, program: None, ..rule("Microsoft Store") };
        let rules = vec![store];
        assert_eq!(coverage(&rules, EXE, &needs(), Category::Private), Coverage::Missing);
    }

    #[test]
    fn wire_names_are_the_frontend_contract() {
        assert_eq!(FirewallStatus::Ok.wire(), "ok");
        assert_eq!(FirewallStatus::NeedsSetup.wire(), "needs-setup");
        assert_eq!(FirewallStatus::Blocked.wire(), "blocked");
        assert_eq!(FirewallStatus::PublicNetwork.wire(), "public-network");
        assert_eq!(FirewallStatus::Managed.wire(), "managed");
        assert_eq!(FirewallStatus::Unknown.wire(), "unknown");
    }
}
