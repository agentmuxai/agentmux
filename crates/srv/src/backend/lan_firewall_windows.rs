// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Read Windows Defender Firewall and the network list, without admin.
//!
//! Read-only COM (`INetFwPolicy2`, `INetworkListManager`): enumerating the rules
//! and reading each connection's category needs no elevation. The decision about
//! what the snapshot means is pure and lives in `lan_firewall`.
//!
//! COM is initialised on a thread of its own and torn down on it, so this never
//! disturbs whichever apartment the calling tokio worker is in.

use std::collections::HashMap;

use windows::core::{Interface, GUID};
use windows::Win32::Foundation::VARIANT_BOOL;
use windows::Win32::NetworkManagement::WindowsFirewall::{
    INetFwPolicy2, INetFwRule, INetFwRule3, NetFwPolicy2, NET_FW_ACTION_ALLOW, NET_FW_MODIFY_STATE_OK,
    NET_FW_RULE_DIR_IN,
};
use windows::Win32::Networking::NetworkListManager::{
    INetworkListManager, NetworkListManager, NLM_ENUM_NETWORK_CONNECTED, NLM_NETWORK_CATEGORY_DOMAIN_AUTHENTICATED,
    NLM_NETWORK_CATEGORY_PRIVATE, NLM_NETWORK_CATEGORY_PUBLIC,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, IDispatch, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
};
use windows::Win32::System::Ole::IEnumVARIANT;
use windows::Win32::System::Variant::VARIANT;

use super::lan_firewall::{Adapter, Category, FwRule, PortSpec, Proto, Remote, Snapshot};

/// Read the firewall rules, the adapters and their network categories.
///
/// Blocking and not cheap (hundreds of rules): call it from a blocking context.
/// Any COM failure is an `Err`, and the caller must treat that as "unknown",
/// never as "covered".
pub fn read_snapshot() -> Result<Snapshot, String> {
    std::thread::Builder::new()
        .name("lan-firewall-read".into())
        .spawn(read_on_this_thread)
        .map_err(|e| format!("could not start the firewall reader thread: {e}"))?
        .join()
        .map_err(|_| "the firewall reader thread panicked".to_string())?
}

fn read_on_this_thread() -> Result<Snapshot, String> {
    // SAFETY: balanced with `CoUninitialize` below, on the same thread.
    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }
        .ok()
        .map_err(|e| format!("COM init failed: {e}"))?;
    let result = read_inner();
    // SAFETY: matches the successful `CoInitializeEx` above.
    unsafe { CoUninitialize() };
    result
}

fn read_inner() -> Result<Snapshot, String> {
    let (rules, local_rules_ignored) = read_rules().map_err(|e| format!("reading firewall rules: {e}"))?;
    let categories = read_categories().map_err(|e| format!("reading network categories: {e}"))?;
    Ok(Snapshot {
        rules,
        adapters: adapters(&categories),
        local_rules_ignored,
    })
}

fn read_rules() -> windows::core::Result<(Vec<FwRule>, bool)> {
    // SAFETY: COM is initialised on this thread by the caller.
    let policy: INetFwPolicy2 = unsafe { CoCreateInstance(&NetFwPolicy2, None, CLSCTX_INPROC_SERVER)? };
    let ignored = unsafe { policy.LocalPolicyModifyState()? } != NET_FW_MODIFY_STATE_OK;
    let collection = unsafe { policy.Rules()? };
    let enumerator: IEnumVARIANT = unsafe { collection._NewEnum()? }.cast()?;

    let mut rules = Vec::new();
    loop {
        let mut item = [VARIANT::default()];
        let mut fetched = 0u32;
        // SAFETY: `item` and `fetched` outlive the call.
        let hr = unsafe { enumerator.Next(&mut item, &mut fetched) };
        if hr.is_err() || fetched == 0 {
            break;
        }
        // A rule that cannot be read is skipped, not fatal: coverage is only
        // ever claimed from rules we could read.
        if let Ok(rule) = rule_from_variant(&item[0]) {
            rules.push(rule);
        }
    }
    Ok((rules, ignored))
}

fn rule_from_variant(v: &VARIANT) -> windows::core::Result<FwRule> {
    let dispatch = IDispatch::try_from(v)?;
    let rule: INetFwRule = dispatch.cast()?;
    // SAFETY: plain property reads on a live COM object.
    unsafe {
        let app = rule.ApplicationName().map(|b| b.to_string()).unwrap_or_default();
        let interface_types = rule.InterfaceTypes().map(|b| b.to_string()).unwrap_or_default();
        let interfaces_set = rule.Interfaces().map(|i| !i.is_empty()).unwrap_or(false);
        let restricted = is_restricted(&rule);
        Ok(FwRule {
            name: rule.Name().map(|b| b.to_string()).unwrap_or_default(),
            enabled: rule.Enabled().unwrap_or(VARIANT_BOOL(0)).as_bool(),
            inbound: rule.Direction()? == NET_FW_RULE_DIR_IN,
            allow: rule.Action()? == NET_FW_ACTION_ALLOW,
            profiles: rule.Profiles()? as u32,
            program: if app.trim().is_empty() { None } else { Some(app) },
            proto: Proto::from_win(rule.Protocol()?),
            local_ports: PortSpec::parse(&rule.LocalPorts().map(|b| b.to_string()).unwrap_or_default()),
            remote: Remote::parse(&rule.RemoteAddresses().map(|b| b.to_string()).unwrap_or_default()),
            // "All" (or empty) means every interface type; anything narrower, or a
            // named-interface list, limits where the rule applies.
            interface_scoped: interfaces_set
                || !(interface_types.trim().is_empty() || interface_types.trim().eq_ignore_ascii_case("all")),
            restricted,
        })
    }
}

/// Is this rule bound to something other than a program path? A Store app, a
/// service, particular users or machines, or secure traffic only. Those rules
/// have an empty `ApplicationName` and so look like "any program". Reads
/// `INetFwRule3` (Windows 8 and later); where that is unavailable the extra
/// restrictions cannot be read, and only the service name is checked.
fn is_restricted(rule: &INetFwRule) -> bool {
    let non_empty = |r: windows::core::Result<windows::core::BSTR>| r.map(|b| !b.to_string().trim().is_empty()).unwrap_or(false);
    // SAFETY: plain property reads on a live COM object.
    unsafe {
        if non_empty(rule.ServiceName()) {
            return true;
        }
        let Ok(r3) = rule.cast::<INetFwRule3>() else { return false };
        non_empty(r3.LocalAppPackageId())
            || non_empty(r3.LocalUserOwner())
            || non_empty(r3.LocalUserAuthorizedList())
            || non_empty(r3.RemoteUserAuthorizedList())
            || non_empty(r3.RemoteMachineAuthorizedList())
            || r3.SecureFlags().map(|f| f != 0).unwrap_or(false)
    }
}

/// Adapter GUID (upper-case, braced) to the category of the network it is
/// connected to.
fn read_categories() -> windows::core::Result<HashMap<String, Category>> {
    // SAFETY: COM is initialised on this thread by the caller.
    let manager: INetworkListManager = unsafe { CoCreateInstance(&NetworkListManager, None, CLSCTX_INPROC_SERVER)? };
    let connections = unsafe { manager.GetNetworkConnections()? };
    let mut out = HashMap::new();
    loop {
        let mut slot = [None];
        let mut fetched = 0u32;
        // SAFETY: `slot` and `fetched` outlive the call.
        let hr = unsafe { connections.Next(&mut slot, Some(&mut fetched)) };
        if hr.is_err() || fetched == 0 {
            break;
        }
        let Some(connection) = slot[0].take() else { break };
        // SAFETY: plain reads on live COM objects.
        let (id, category, connected) = unsafe {
            let id = connection.GetAdapterId()?;
            let network = connection.GetNetwork()?;
            (id, network.GetCategory()?, connection.IsConnected()?.as_bool())
        };
        if !connected {
            continue;
        }
        let category = match category {
            c if c == NLM_NETWORK_CATEGORY_PRIVATE => Category::Private,
            c if c == NLM_NETWORK_CATEGORY_DOMAIN_AUTHENTICATED => Category::Domain,
            c if c == NLM_NETWORK_CATEGORY_PUBLIC => Category::Public,
            _ => Category::Public,
        };
        out.insert(guid_string(&id), category);
    }
    let _ = NLM_ENUM_NETWORK_CONNECTED;
    Ok(out)
}

fn guid_string(g: &GUID) -> String {
    format!(
        "{{{:08X}-{:04X}-{:04X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}}}",
        g.data1, g.data2, g.data3, g.data4[0], g.data4[1], g.data4[2], g.data4[3], g.data4[4], g.data4[5], g.data4[6], g.data4[7]
    )
}

/// One entry per adapter that has a usable IPv4 address, with its category.
fn adapters(categories: &HashMap<String, Category>) -> Vec<Adapter> {
    let mut by_id: std::collections::BTreeMap<String, Adapter> = std::collections::BTreeMap::new();
    for iface in if_addrs::get_if_addrs().unwrap_or_default() {
        let std::net::IpAddr::V4(v4) = iface.ip() else { continue };
        if v4.is_loopback() || v4.is_unspecified() || v4.is_link_local() {
            continue;
        }
        let id = iface.adapter_name.to_uppercase();
        let entry = by_id.entry(id.clone()).or_insert_with(|| Adapter {
            category: categories.get(&id).copied(),
            id,
            name: iface.name.clone(),
            ipv4: Vec::new(),
        });
        entry.ipv4.push(v4);
    }
    by_id.into_values().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guid_strings_match_the_adapter_name_format() {
        let g = GUID::from_values(0x1234ABCD, 0x00EF, 0x0123, [0x45, 0x67, 0x89, 0xAB, 0xCD, 0xEF, 0x01, 0x23]);
        assert_eq!(guid_string(&g), "{1234ABCD-00EF-0123-4567-89ABCDEF0123}");
    }

    /// Whatever this machine's firewall looks like, reading it must either work
    /// or fail cleanly. It must never panic and never invent a snapshot.
    #[test]
    fn reading_this_machine_never_panics() {
        match read_snapshot() {
            Ok(snap) => {
                for a in &snap.adapters {
                    assert!(!a.ipv4.is_empty());
                }
            }
            Err(e) => assert!(!e.is_empty()),
        }
    }

    /// Manual: prints what the firewall reader sees, so the verdict can be
    /// compared with the real machine. `cargo test -p agentmux-srv -- --ignored --nocapture reads_and_reports`
    #[test]
    #[ignore = "reads the real firewall; run by hand"]
    fn reads_and_reports() {
        let snap = read_snapshot().expect("snapshot");
        println!("rules: {} (local rules ignored: {})", snap.rules.len(), snap.local_rules_ignored);
        for a in &snap.adapters {
            println!("adapter {} {} {:?} {:?}", a.id, a.name, a.ipv4, a.category);
        }
        // Optionally judge another binary, e.g. a running srv:
        // AGENTMUX_FW_PROBE_EXE=<path> AGENTMUX_FW_PROBE_PORTS=<web>,<ws>
        let exe = std::env::var("AGENTMUX_FW_PROBE_EXE")
            .unwrap_or_else(|_| std::env::current_exe().unwrap().to_string_lossy().to_string());
        let (web, ws) = std::env::var("AGENTMUX_FW_PROBE_PORTS")
            .ok()
            .and_then(|v| {
                let (a, b) = v.split_once(',')?;
                Some((a.trim().parse::<u16>().ok()?, b.trim().parse::<u16>().ok()?))
            })
            .unwrap_or((29700, 29701));
        println!("judging {exe} on ports {web},{ws}");
        let mine = snap
            .rules
            .iter()
            .filter(|r| r.program.as_deref().is_some_and(|p| super::super::lan_firewall::same_path(p, &exe)))
            .count();
        println!("rules naming this test binary: {mine}");
        let report = super::super::lan_firewall::report(&snap, &exe, &super::super::lan_firewall::lan_needs(web, ws));
        println!("status for this binary: {}", report.status.wire());
        for need in super::super::lan_firewall::lan_needs(web, ws) {
            for (allow, label) in [(true, "allow"), (false, "BLOCK")] {
                let hits = super::super::lan_firewall::matching_rules(
                    &snap.rules,
                    &exe,
                    need,
                    Category::Private,
                    allow,
                );
                for r in hits.iter().take(4) {
                    println!(
                        "  need {:?}:{} {label} <- {:?} program={:?} ports={:?} remote={:?}",
                        need.proto, need.port, r.name, r.program, r.local_ports, r.remote
                    );
                }
                if hits.len() > 4 {
                    println!("  need {:?}:{} {label} ... and {} more", need.proto, need.port, hits.len() - 4);
                }
            }
        }
        for (a, s) in &report.per_adapter {
            println!("  {} -> {:?}", a.name, s);
        }
    }
}
