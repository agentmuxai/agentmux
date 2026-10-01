# SPEC: LAN that works on a fresh install, with no manual firewall steps

**Date:** 2026-10-01
**Status:** proposed — research and design; nothing implemented. Replaces the "keep LAN off to avoid the prompt" stance of [`windows-firewall-fix.md`](windows-firewall-fix.md), which is now marked historical; the opt-in default itself is unchanged.
**Author:** AgentX (narko), at the owner's request
**Affects:** `crates/srv/src/backend/lan_listeners.rs`, `lan_discovery.rs`, `crates/srv/src/bootstrap/network.rs`, the status-bar LAN indicator and HostPopover, `packaging/windows/agentmux.iss`.

## 1. What went wrong (2026-09-30)

The owner enabled LAN on narko (Windows) and on Area54. Area54 saw narko; narko never saw Area54, and no LAN jekt could be sent. Both ends were running v0.59.1 with LAN discovery on.

- narko's srv started its listeners and registered its mDNS service (`LAN listener started`, `LAN discovery registered mDNS service`) and logged **no warning of any kind**. It simply never resolved a peer.
- Windows Firewall held, for `agentmux-srv-0.59.1-windows.x64.exe`, two inbound Allow rules (TCP and UDP), both scoped to the **Public** profile only. Those are what Windows' "Security Alert" prompt creates, with the profile boxes that were ticked at that moment.
- narko's only LAN adapter (`Ethernet`, network `asaf_5G`) is on the **Private** profile. A Public-only rule does not apply there, so Windows dropped all inbound traffic to the srv: Area54's mDNS announcements (UDP) and any inbound connection (TCP).
- narko still announced itself (outbound is allowed), which is why Area54 saw narko. The asymmetry was the whole symptom.
- The status bar showed LAN as **idle** ("on, nobody out there"), the documented-healthy state. Nothing told the owner it was the firewall.

The machine also held **606** AgentMux firewall rules, one TCP/UDP pair per dev or portable build path.

The owner approved the OS prompt. That is the point: **the prompt cannot be relied on.** It is OS-controlled, its default profile follows the current network, a cancel or a non-admin answer silently creates a *block* rule, and it has to be answered again for every new exe path.

## 2. What the research says

| # | Finding | Source |
|---|---|---|
| R1 | If an existing rule matches the program, Windows shows **no** dialog. The robust pattern is to register the rule *before* the app first listens, not to depend on the dialog. | [comcomponent: installer registration](https://comcomponent.com/en/blog/windows-firewall-business-apps/) |
| R2 | If the dialog is cancelled, or answered by a non-administrator, Windows creates a **block** rule (typically one TCP, one UDP). Block overrides allow, so a later allow rule does not help unless the block is removed. | same |
| R3 | Profiles: Domain, Private (an administrator marks a network trusted), Public (the default for unidentified networks). *"A rule limited to Domain and Private does not apply if the traffic belongs to the Public profile."* Check with `Get-NetConnectionProfile`. | same; [Syncthing forum](https://forum.syncthing.net/t/simplewall-or-windows-firewall-rules-for-syncthing-to-connect-only-on-private-network/18515) |
| R4 | A program rule is bound to the exe **path**. When the path changes, the rule stops matching: keep the path fixed or delete and re-register. | comcomponent |
| R5 | Scope narrowly: exact program path, profile, protocol, and `remoteip=LocalSubnet`. Use a fixed rule *Name* separate from the DisplayName so re-registration is idempotent. | comcomponent; [Microsoft netsh docs](https://learn.microsoft.com/ja-jp/troubleshoot/windows-server/networking/netsh-advfirewall-firewall-control-firewall-behavior) |
| R6 | **Elevated-helper pattern** for a per-user, non-elevated app that listens on LAN (a design proposal for a project in exactly our position): the same exe gets a `--configure-firewall` mode, launched with the `runas` verb; it adds or updates **one** inbound allow rule scoped to the program, for Private and Domain, **removes any block rules** for that program, uses the `INetFwPolicy2` COM API, is idempotent, and exits with clear codes. One sentence of explanation before one UAC prompt, and the choice is remembered. Status is readable **without** admin. | [Weir issue #801](https://github.com/jampat000/Weir/issues/801) |
| R7 | A running process cannot be elevated; only a new process can. So the main app stays unelevated and relaunches itself for the one privileged step. A repeating UAC prompt (Docker Desktop's reboot prompt) is the failure to avoid: do it once, make it idempotent, remember it. | Weir #801; [docker/for-win #13806](https://github.com/docker/for-win/issues/13806) |
| R8 | **mDNS on Windows 10 1703+** is handled by the DNS Client service, and the built-in *"mDNS (UDP-In)"* rule is scoped to that service (`svchost`), not to apps. The Win32 DNS-SD APIs (`DnsServiceRegister`, `DnsServiceBrowse`, desktop apps, Windows 10+) go through it. **Inference, not stated by any source:** an app using them needs no per-app UDP 5353 rule. Chrome, Edge and Teams ship their own responder and so need their own rules. | [Microsoft: mDNS in the enterprise](https://techcommunity.microsoft.com/t5/networking-blog/mdns-in-the-enterprise/ba-p/3275777); [DnsServiceBrowse](https://learn.microsoft.com/en-us/windows/win32/api/windns/nf-windns-dnsservicebrowse) |
| R9 | Peers like Syncthing and KDE Connect document **fixed ports** (22000, 21027; 1714–1764) and Private-profile rules. We cannot: our srv ports are OS-chosen per run. | [Syncthing firewall docs](https://docs.syncthing.net/users/firewall.html) |
| R10 | **macOS 15+** needs `NSLocalNetworkUsageDescription` and `NSBonjourServices`, else it fails closed with no prompt. Caveat: raw UDP multicast sockets may not trigger the prompt; the system Bonjour API does. | [Apple forum thread](https://developer.apple.com/forums/thread/814226); [NVIDIA/Personal-AI-Router #3](https://github.com/NVIDIA/Personal-AI-Router/issues/3) |
| R11 | Managed machines (Group Policy / Intune with local rule merging off) ignore locally added rules. Do not work around it; say so. | comcomponent |

## 3. Where we stand

- srv binds **loopback, OS-chosen ports** at startup and `LanListenerSupervisor` adds LAN listeners on the *same* ports when LAN is enabled (`bootstrap/network.rs`, `lan_listeners.rs`). With ports chosen per run, no port rule can be written, and a program rule is the only option today (R9). Section 4.1 changes the ports so that it stops being the only option.
- mDNS uses the `mdns-sd` crate: raw sockets on UDP 5353, i.e. exactly the app-owned responder that needs its own UDP rule (R8).
- Windows ships three ways: **portable zip** (no installer), **Inno Setup per-user installer** (`packaging/windows/agentmux.iss`, no admin), and **MSIX**. A fresh install has no admin and no rules.
- LAN is **opt-in** (HostPopover toggle). `windows-firewall-fix.md` kept the prompt away by keeping mDNS off. That avoided the dialog but left LAN unusable until the owner hunted for a toggle, and gave no help when it broke.
- macOS already declares both Local Network keys (`scripts/package-macos.sh:386`). Whether raw-socket mDNS triggers the prompt is unverified (R10).
- **The sidecar's filename embeds its version** (`runtime\agentmux-srv-0.59.1-windows.x64.exe`), and portable and dev builds live at a new path each time. A program rule therefore stops matching on every update, and re-registering it needs another elevation: the repeating-prompt failure R7 warns about. This is why the first draft of 4.1 (one rule per exe path) cannot deliver "one prompt, ever".
- The indicator has `off | idle | peers | error`. A firewall block lands in `idle`, the "healthy" state.

## 4. Design

### 4.1 Fixed LAN ports, and port rules that survive updates

**Core (cross-platform).** srv chooses its web and ws listener ports from a fixed range, `29700..=29799` (first free pair). **The range must sit below 32768:** the OS hands ephemeral ports to outbound connections, and the defaults are 49152-65535 on Windows and macOS but **32768-60999 on Linux**, so a port in either range can be busy on a LAN address while free on loopback, and the startup check only tests loopback (found in review of the first draft, which used `47892..=47991`). The block is also chosen away from well-known neighbours (Syncthing, Synergy, Minecraft, Steam, MongoDB, Kubernetes NodePorts); see `lan_ports.rs`. The loopback and LAN listeners keep sharing a port, as `lan_listeners.rs` requires, and mDNS advertises the actual port. Several instances on one host (channels, portable builds, `task dev`) take successive pairs, so the range holds about fifty. If the range is exhausted srv falls back to an OS-chosen port, and the indicator reports `needs-setup` because that port is not covered by the rule. Headless mode keeps `--web-port` / `--ws-port`.

**Windows rules.** A small mode of the signed host exe (proposed: `agentmux.exe --configure-lan-firewall`, so the UAC prompt names AgentMux) does, via `INetFwPolicy2`, in **one** elevated run:

1. Ensure **two** inbound Allow rules with fixed names, **not tied to any program**: `AgentMux LAN (TCP)` on local ports `29700-29799`, and `AgentMux LAN (UDP)` on `5353,47891` (mDNS and the broadcast fallback). Profiles **Private + Domain**, `RemoteAddresses = LocalSubnet`, edge traversal off. (Two rules because a port range needs a concrete protocol.)
2. **Delete any Block rules** whose program is an AgentMux binary (left behind by a cancelled prompt, R2).
3. **Prune** the legacy per-program AgentMux rules (the 606).
4. If the user accepted a Public network (4.4), add the scoped Public copies.
5. Exit codes: `0` ok, `1223`-style cancel, `2` policy-managed (R11), `3` other. Idempotent; running it twice changes nothing.

Port rules are what make the base setup **once per machine**: they match through updates, new builds at new paths, and reinstalls (R4, R7). The count is exact, not "one ever": **one prompt per machine for Private and Domain networks, and one more each time the user chooses to trust a new Public network** (4.4), because each of those is a deliberate decision that adds a rule. The cost is that any local program that listens in the range is reachable from the subnet. The range is small, `LocalSubnet` narrows who can reach it, and the LAN routes are still gated by the scoped LAN key. Predictable *loopback* ports are no new exposure, since they are authenticated already.

Public is **not** in the base rules; see 4.4.

### 4.2 Order: rule first, then listeners (R1)

Enabling LAN becomes:

1. Read the firewall state **without admin** (`INetFwPolicy2` read, `INetworkListManager` for profiles).
2. **Per interface, not per machine.** `LanListenerSupervisor` binds every non-loopback address, and each adapter has its own connection profile. For each interface it would bind, require an *applicable* allow rule: the Private/Domain rules cover an adapter only if that adapter's profile is Private or Domain, and a Public adapter is covered only by a consented, interface-scoped Public copy (4.4). Also require that srv's ports are inside the range and that no block rule exists. Bind the LAN listeners **only on the covered interfaces**, and start mDNS only on the covered interfaces (see the caveat below). **No OS dialog can appear** for a covered interface, because a matching allow rule exists. An uncovered interface simply stays unbound; the indicator reports why (`needs-setup` or `public-network`, 4.3). This also keeps sockets off a Public adapter that nobody agreed to trust.
**Caveat, verified in the source: mDNS cannot be limited to covered interfaces *after* the daemon exists.** `mdns-sd` 0.12.0 (our version) binds a socket on **every** interface when the daemon is constructed (`Zeroconf::new`, `service_daemon.rs:899-945`, via `my_ip_interfaces()` and `new_socket_bind`). `disable_interface` / `enable_interface` only close or reopen sockets afterwards, so selecting interfaces after construction cannot prevent a first bind on an uncovered one. The bind on an uncovered interface is what could raise Windows' alert. PR B therefore has to pick one of:

- **(a)** construct the daemon only when *every* non-loopback interface it would bind is covered (safe, but a host with an uncovered virtual adapter, such as VMware or WSL, would then get no mDNS at all);
- **(b)** use a version or patch of `mdns-sd` that applies interface selection *before* the first bind;
- **(c)** discover over our own per-interface sockets (the UDP broadcast fallback already is one).

Which is needed depends on a fact not yet measured: does binding on an uncovered interface raise the alert *when an allow rule already exists for another profile*? That is measured on the clean Windows machine first (section 6, and acceptance case 8), before choosing.

3. Otherwise show one explanatory sentence ("AgentMux will ask Windows for permission to accept connections from other devices on your private network"), then launch the helper with `runas` (R6, R7). On success go to step 2. On cancel: stay loopback-only, show "LAN needs one-time setup" (4.3), do **not** retry on every start.

The srv must not bind a non-loopback socket before step 2, or Windows raises its own dialog and, on cancel, plants a block rule.

### 4.3 The indicator must tell the truth

Extend `resolveLanIndicator` with states for what a user can act on:

| State | When | Message / action |
|---|---|---|
| `needs-setup` | LAN on, rule missing or points at another path | "LAN needs one-time setup" → run the helper |
| `blocked` | a Block rule exists for AgentMux | "Windows is blocking AgentMux on this network" → helper removes it |
| `public-network` | the only connected adapter(s) are Public | "Windows treats this network as Public, so incoming connections are blocked. Mark it Private" + `ms-settings:network` |
| `managed` | policy ignores local rules | "Your administrator manages the firewall for this device" |
| `undiscoverable` | LAN on, firewall fine, but this instance holds no usable IPv4 mDNS socket (4.7), so no other machine can find it | "Other machines can't see this one. Another program may be using the mDNS port (5353)" -> Retry |
| `idle` | rule OK, profile OK, no peers | unchanged |

A silent `idle` with a blocked firewall (section 1) is the bug this removes.

### 4.4 Public networks need the user's consent, per network

A new Windows network is typically classed **Public**, and many home users never change it, so a Private-only rule would leave LAN dead on a default install. But opening Public silently is wrong: the scoped LAN key is broadcast in the mDNS TXT record (`bootstrap/network.rs`), so anyone on the segment, a café for instance, could read it and call the LAN routes.

So, when the active LAN adapter is Public (detected without admin through `INetworkListManager`):

1. An in-app screen: *"Windows treats 'asaf_5G' as a Public network, which blocks incoming connections. Trust this network for AgentMux LAN? [Trust this network] [Not now]"*, naming the network and its subnet.
2. Accepting adds Public-profile copies of the two rules **scoped to that adapter and that network's subnet** (`Interfaces` plus `RemoteAddresses = <CIDR>`). The network (name and CIDR) is recorded so the UI can show and revoke it. This needs an elevated run of its own unless the base rules are being installed at the same moment (first enable on a Public network does both in one prompt). Trusting a *later* Public network is another prompt, by design: it is a deliberate decision, and the alternative, a persistent privileged service, is a much larger attack surface than one extra UAC click. The residual risk is stated on the screen: the same adapter on a *different* network that happens to use the same private range (192.168.1.0/24 is common) would match.
3. Declining keeps AgentMux loopback-only and the indicator reads `public-network`, with a link to Windows' network settings. Marking the network Private in Windows also works, but it changes more than AgentMux needs (it turns on file and printer sharing and network discovery), so it is offered as an alternative and is not the default.

It is never done silently.

### 4.5 Installers

- **Inno per-user installer:** an optional task, "Allow LAN access (asks for administrator permission)", that runs the helper with `runas`. Unticked or declined, the first LAN enable does it (4.2). Uninstall runs the helper's remove mode.
- **Portable / dev builds:** no installer, so 4.2 is the whole mechanism. The rules are port-based, so a new build at a new path needs **nothing**.
- **MSIX:** verify separately whether the package identity changes the prompt behaviour; not assumed here.

### 4.6 Hardening, phase 2 (separate PRs)

- **Windows mDNS through the OS** (`DnsServiceRegister` / `DnsServiceBrowse`): inbound mDNS goes through the DNS Client service's built-in rule, so discovery would not depend on our UDP rule at all (R8). Our TCP connection to a peer still needs 4.1. Must be verified on a clean machine before relying on it; today it is an inference.
- **macOS:** confirm on a clean macOS 15 machine that `mdns-sd` raw sockets raise the Local Network prompt; if not, discover through the system Bonjour API (R10).
- **Linux:** detect an active `ufw` or `firewalld` and show the exact one-line command; do not silently modify it.

### 4.7 mDNS must not fail silently (found on Area54, 2026-10-01)

`mdns-sd` 0.12 (our version) creates one socket per interface when the daemon is built. For IPv4 that is a bind to `0.0.0.0:5353` (address reuse on), a multicast-group join and one empty test packet; if any step fails, the daemon **logs at `debug!` only and skips that interface** (`service_daemon.rs`, "bind a socket to {}: {}. Skipped."). Nothing appears at the default log level, and `LAN discovery started (mDNS)` is still logged. The result is an instance that **hears** peers and is **never heard**: the worst kind of one-way failure, with a correct firewall and a healthy-looking indicator.

Requirements:

1. **Check the daemon's own announcements, with no probe at all.** Two obvious probes do not work. A socket bound to UDP 5353 competes with the daemon it is testing (Windows lets a later address-reuse socket take traffic; two same-process mDNS receivers on macOS can lose events, see `docs/retro/retro-macos-ci-mdns-multicast-unsupported-2026-08-12.md`; Codex P1 on #4133), and a legacy-unicast question from an ephemeral port is never answered, because `mdns-sd` 0.12 **always replies by multicast** (`multicast_on_intf`, `service_daemon.rs`), so the reply never reaches the asking socket. What the library does offer is its monitor channel: `DaemonEvent::Announce` fires only for interfaces that have a working socket. Take the monitor **before** registering, collect the IPv4 addresses announced on, and compare them with the IPv4 addresses a peer could reach us on (not loopback, link-local, multicast). Announced on none of them is `undiscoverable` (4.3); on some, `degraded` (a warning, often a virtual adapter); on all, healthy. The check opens no socket and sends no packet. Implemented in `backend/lan_mdns_health.rs`.
2. **Recover automatically, but not on a single miss.** A watchdog looks every 15 s. One undiscoverable verdict can be a slow start, so it takes two in a row, then rebuilds the daemon (the same path as switching LAN off and on, which fixed Area54), up to three times, and says so once when it gives up. The budget is restored only by full health. A `Pending` verdict (the 5 s start-up grace) is neither a strike nor a recovery. The indicator is told through `laninstances:health` and shows `undiscoverable` ahead of `peers`: hearing peers does not prove being heard.
3. **A path that does not depend on mDNS.** Desktop-to-desktop discovery has no second route today: the UDP broadcast responder on 47891 only answers mobile probes (`lan_discovery.rs`). A broadcast announce between srv instances would cover hosts where mDNS is blocked or contested. This is larger than 1 and 2, so it is its own PR. Note 47891 lies in Linux's default ephemeral range (see 4.1), so it should move into the fixed block.

## 5. Acceptance (a fresh machine is the test)

On a clean Windows 11 VM with no AgentMux rules, per-user install, standard (non-admin) login that can approve UAC:

1. Enable LAN on a Private network → one explanatory sentence, **one** UAC prompt, **no** "Windows Security Alert". Within 60 s the peer list fills **in both directions**, and a LAN jekt arrives `lan-verified`.
2. Decline UAC → AgentMux keeps working loopback-only; the indicator reads `needs-setup`; no prompt on the next start.
3. Cancel the *Windows* dialog in a build that predates this (block rule present) → helper removes the block; LAN works.
4. Network classed Public → the in-app consent screen appears. Accepting adds the scoped rules and LAN works, in the same UAC prompt if this is the first enable, or in **one more prompt** if the base rules already exist. A second, different Public network costs one more prompt each. Declining leaves loopback-only with the `public-network` indicator and a Settings link.
5. **An update, or a second build at a new path → LAN keeps working with no prompt and no new rule** (the port rules still match; the rule count does not grow).
6. Uninstall → no AgentMux rule remains.
7. Two machines, one per direction, exchange a plain and a keyword jekt over LAN (done for Windows ↔ macOS on 2026-10-01, section 8).
8. **Mixed profiles:** a host with one Private and one Public adapter, base rules installed, Public not trusted → LAN listeners bind on the Private adapter only, no Windows Security Alert appears, and the indicator names the uncovered adapter.
9. Range exhausted → srv falls back to an OS-chosen port, nothing crashes, and the indicator reads `needs-setup`.
10. A Linux guest on a **bridged** adapter appears as a normal peer; on **NAT** it does not, and the docs say so (section 6).

## 6. Open questions

- Which binary does the UAC prompt show? It should be the signed host exe, not the sidecar, so the publisher reads AgentMux.
- Is `RemoteAddresses = LocalSubnet` enough on this fleet, or do some hosts (VPN, WSL, Hyper-V adapters, as on narko) need an explicit range? Narko lists four LAN listeners, three on virtual adapters.
- IPv6 link-local: `Area54.local` resolved to an `fe80::` address. Confirm the rule and the listeners cover it.
- **Does an uncovered interface raise the alert once an allow rule exists for another profile?** Windows decides when to show its Security Alert, and the documented behaviour (R1) only says an existing *matching* rule suppresses it. If it does fire, 4.2 needs option (a), (b) or (c); if not, the per-interface listener gate alone is enough and mDNS can stay as it is. Narko already runs four LAN listeners, three on virtual adapters, so this is a real configuration, not a corner case. Measure it first on the clean Windows machine.
- **Virtual machines.** A guest is a normal LAN member only on a **bridged** adapter (ideally wired). On NAT, mDNS multicast does not leave the host's private subnet and other machines cannot reach the guest; host-only is isolated. Check the hypervisor's adapter mode before diagnosing a guest. Bridging over Wi-Fi is unreliable on some hypervisors.
- **The UDP broadcast fallback port `47891` is itself inside Linux's default ephemeral range**, so on Linux it can fail to bind while an outbound connection holds it. It predates this spec and is UDP-only; whether to move it is a separate question, and the rule keeps naming it until then.
- Port-range collisions: fifty instances per host is plenty for a person, but a CI or test host running many `task dev` builds could exhaust it; the OS-chosen fallback plus `needs-setup` is the safety net, but confirm the failure is legible.
- The product currently treats a failed LAN start as an error only when the mDNS daemon itself fails; confirm what `lan_discovery_error` should add.

## 7. Immediate unblocker (not the fix)

Until 4.1 exists, add the Private profile to the existing rule (Windows Security → Firewall → Allow an app → `agentmux-srv-…` → tick Private), or run, elevated:

```powershell
netsh advfirewall firewall add rule name="AgentMux LAN" dir=in action=allow protocol=any profile=private,domain remoteip=localsubnet program="<full path to agentmux-srv.exe>"
```

It has to be repeated for every new build path, which is exactly why 4.1 exists.

## 8. Evidence from the LAN tests (2026-10-01)

narko (Windows 11, v0.59.1) and starpower (macOS 26.5.2, v0.59.1), LAN only, default OS settings on the macOS side:

| Check | Result |
|---|---|
| narko → starpower, plain | `DELIVERY=lan`, `TRUST=lan-verified`, `TIER=coord` |
| starpower → narko, plain | `DELIVERY=lan`, `TRUST=lan-verified`, `TIER=coord`, within seconds |
| keyword, starpower → narko | `TIER=sensitive`, `TRUST=lan-verified`, `ESCALATE=none`, informational banner only |
| keyword, narko → starpower | same on the receiving side |
| discovery | both directions; each host lists the other (narko `192.168.1.230:57319`, starpower `192.168.1.195`) |

Not yet known: whether macOS showed a Local Network permission prompt on starpower (the agent cannot see the screen; the operator must say), and how LAN was enabled there. **charlie** (a Linux VM on gamerlove, see section 6) was not on narko's LAN list at that point; once its adapter was bridged it joined, and a plain jekt in each direction was `DELIVERY=lan`, `TRUST=lan-verified` (charlie v0.58.2 with the 0.59.1 machines). **Area54** (Windows, wired) stayed one-way until the fault in 8.1 was cleared.

The same session also showed the failure this spec exists for: narko saw nothing while Area54 saw narko, until the Private profile was added by hand.

### 8.1 Area54: heard, never heard (root-cause analysis, 2026-10-01)

Symptom: Area54 listed narko, starpower and charlie; none of them listed Area54. LAN was enabled on all four. Worked out with Manoz (the Area54 agent) from facts on each side; nothing was changed on Area54 until the last step.

| Finding | Evidence |
|---|---|
| Not the firewall, not the network | Area54 -> narko jekts arrived `DELIVERY=lan`; TCP from narko to `192.168.1.26:65524` opened; Area54 has Private-profile allow rules for the running srv; wired, same router and profile as the others |
| Area54's multicast reaches the others | A beacon run on Area54 (a standard mDNS question every 3 s from its Ethernet adapter) was answered by narko, starpower and charlie, and narko captured about 35 queries from `.26` in that window |
| srv on Area54 never **answers** | About 35 queries and **zero responses** from `.26`, not even for its own registered service; narko's log has no "LAN peer discovered" for Area54 in 13 hours while starpower and charlie appear hourly; narko's own multicast question was answered by starpower and charlie but not by Area54 |
| srv held **no IPv4 socket on UDP 5353** | `Get-NetUDPEndpoint -LocalPort 5353`: srv only on `::`, Chrome on `0.0.0.0`. On narko, which works, srv is on `0.0.0.0` |
| Not a port conflict at start, as first suspected | srv started 2026-09-30 21:41Z and LAN was enabled at runtime at 03:15:20Z; Chrome's oldest process started 08:37Z, later; a plain address-reuse bind, group join and test send to `0.0.0.0:5353` succeeded on both IPv4 interfaces afterwards |
| Not an interface change or sleep | no network connect or disconnect events since 2026-09-27, no sleep since May, last boot 2026-09-13 |
| **Clearing it** | LAN switched off and on at 16:20:39Z and 16:20:46Z (a brand-new mDNS daemon, same srv, no restart): srv then held `0.0.0.0:5353`, narko discovered Area54 at once with `192.168.1.26`, all four machines listed each other, and jekts in both directions were `DELIVERY=lan`, `TRUST=lan-verified` |

What is **proven**: the IPv4 socket was missing, its absence alone explains every symptom, and rebuilding the daemon restored it. What is **not**: what made the IPv4 bind fail at 03:15:20Z (a process that held the port then and has since gone, or a Windows multicast quirk). One further observation is also unexplained: after the toggle srv held the IPv4 socket and had *lost* the IPv6 one to a Chrome process, so on this machine each address family seems to end up with srv or Chrome, not both. Neither changes the requirement: the failure is silent and the indicator cannot see it (4.7).

### 8.2 The firewall reader, checked against real rules (2026-10-01)

Reading the firewall needs no admin: `INetFwPolicy2` for the rules and `INetworkListManager` for each connection's category, joined to adapters by GUID (`backend/lan_firewall_windows.rs`, read-only). On narko it read 1,286 rules in about 60 ms and mapped the Ethernet adapter to its Private profile; the VMware and WSL adapters have no connected-network category and come out `unknown`, which is why one covered adapter is enough for `ok` (`lan_firewall::overall`).

Judged against the real rules, the decision logic gave the answer the spec predicts:

| Binary | Rules naming it | Verdict |
|---|---|---|
| the running 0.59.1 srv (LAN works, rules added by hand on 2026-09-30) | 2 (TCP, UDP) | covered, and exactly those two rules were the evidence |
| the 0.58.2 build also on narko | 0 | `needs-setup` (R7: path-bound rules do not follow a new path) |
| a freshly built test binary | 0 | `needs-setup` |

**What the real data found that the design had not anticipated.** The first version reported Ethernet as *covered* for a binary no rule named. Twenty-five rules ("Solitaire & Casual Games", "Microsoft Store", ...) have an empty `ApplicationName` and ports `*`, so they look like "any program", but they are bound to a Store app package. The same applies to rules bound to a service, to particular users or machines, or to secure (authenticated) traffic only. The reader now marks any such rule `restricted` and the decision skips it (`FwRule::restricted`). Without this a machine with ordinary Store apps would never be told it needs setup, which is the failure this spec exists to remove. It is a test (`a_rule_bound_to_a_service_or_store_app_is_not_an_any_program_rule`), but it was found by running the reader on a real machine, not by thinking about the cases.

**The profile's own switches count too** (ReAgent P1 on #4151). Judging from rules alone reports `needs-setup` on a profile whose firewall is off or whose default inbound action is Allow, where no rule is needed. The reader therefore also reads, per profile, whether the firewall is on, whether "block all incoming connections" is set (it overrides every allow, so it is `blocked`), and the default inbound action. A profile with the firewall off filters nothing, block rules included.

**Which rules are skipped as "not for us" is empirical.** Marking every rule with an owner SID as restricted was wrong: a user-created Allow rule can carry one (ReAgent P1 on #4151). Dumping the properties on narko showed the actual difference: the Store-app rules have an owner SID and **no program path**, no package id and no authorisation list; the rules for `agentmux-srv` have a program path and no owner. So an owner counts as a restriction only on a rule that names no program (a per-user rule). That is a measurement on one machine, not documented Windows behaviour, and it is the first thing to recheck on the clean machine. `reads_and_reports` and `dump_rule_properties` (`--ignored`) are the tools.

**What the rule is judged against.** The inbound traffic a peer needs is our web and ws ports (TCP), mDNS (UDP 5353) and the UDP broadcast-probe responder (47891, which the spec's own UDP rule already lists; Codex P2 on #4151). A rule counts for an adapter only if its **local addresses** include one of that adapter's addresses: a rule limited to one local IP must not make every adapter in the profile look covered, nor an address-scoped block look like a global one (Codex P2 on #4151).

**Unreadable means unknown, never covered** (Codex P2 on #4151, twice). Every COM read that decides a verdict leans the cautious way when it fails. A rule whose ports, remote or local addresses, interface scope, program or enabled flag cannot be read is *counted* as unreadable instead of being given a permissive default (an empty string reads as "unrestricted" in every parser), and a snapshot with any unreadable rule cannot report `ok`: it reports `unknown`, because the rule it could not read may be the block that overrides a visible allow. A failure part-way through enumerating the rules, or the network connections, is an error for the whole read, not the end of the list, so the watcher publishes `unknown` instead of acting on a partial snapshot. **The same rule applies to a block we cannot evaluate** (ReAgent on #4151). For an allow, skipping a rule we cannot judge is the cautious choice; for a block it leans toward `ok`, which is the direction this rule forbids. A rule therefore answers *applies*, *might apply* or *does not*: a block whose evaluable fields all match but which is scoped to an interface type, bound to a service or user, or carries a port or address spec we could not read is *might apply*, and next to a visible allow it downgrades the adapter to `unknown` instead of `covered`. A block that something we can evaluate rules out (disabled, outbound, another profile or program, ports or addresses that exclude ours, or limited to specific remote addresses) is still ignored. The problem verdicts stand even so: a rule we could not read does not make a missing allow less missing. On narko all 1,290 rules read, so none of this changes its verdict; it matters on a machine with an unusual rule.

**Indicator order.** off, then undiscoverable, then **every firewall verdict** (`blocked`, `needs-setup`, `public-network`, `managed`), then peers, then a start-up error, then idle. An earlier draft ranked peers above the inferred firewall states, on the argument that seeing a peer proves discovery works. That is wrong, and it is the failure this spec exists for: a peer count proves only that this process *received* discovery traffic, not that anyone can connect to its listeners (Codex P1 on #4151). So a problem outranks peers; when peers exist the label keeps both facts ("2 on LAN. ...") and the glyph stays filled. The cost is that a verdict inferred from a rule we cannot read can show a warning on a LAN that works; the wording therefore says what is missing, not that LAN is broken.

## 9. Delivery plan

One spec, separate PRs, in this order. Windows is the failing path and should not wait on macOS or Linux verification; each platform needs its own hardware to prove it.

| PR | Scope | Verified on |
|---|---|---|
| **A: core** | Fixed LAN port range with fallback; advertise the actual port; a firewall-status interface that reports coverage **per interface**; the new indicator states (`needs-setup`, `blocked`, `public-network`, `managed`, `undiscoverable`), and the mDNS self-probe and automatic daemon rebuild of 4.7 (items 1 and 2). No OS-specific code. | unit tests; narko ↔ starpower |
| **B1: Windows, read-only detection** | The firewall reader, the pure coverage decision, the watcher that publishes `laninstances:firewall`, and the status-bar states `blocked`, `needs-setup`, `public-network`, `managed` (4.3). Changes no rule and gates nothing. Done in the PR that adds this line; checked against narko's real rules (8.2). | narko |
| **B2: Windows, setup and gating** | The elevated helper, the two port rules, Public-network consent, per-interface coverage gating of the LAN listeners and a decision on mDNS interface selection (4.2 caveat, settled by the measurement in section 6), installer step and uninstall cleanup. Needs the clean Windows machine. | a fresh Windows machine |
| **C: macOS** | Verify first; change code only if the Local Network prompt does not appear with our current discovery. | starpower |
| **E: discovery fallback** | A desktop-to-desktop announce that does not depend on mDNS (4.7 item 3); moves the UDP port into the fixed block. | a host where another program holds UDP 5353 |
| **D: Linux** | Detect an active `ufw` or `firewalld` and show the exact command. | charlie, once it is a bridged LAN member |

Separate from this spec: WAN delivery has no catch-up pull (a message that arrives before an agent subscribes waits for the next unrelated wake). Tracked on its own.
