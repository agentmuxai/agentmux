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

**Core (cross-platform).** srv chooses its web and ws listener ports from a fixed range, `47892..=47991` (first free pair; the existing UDP broadcast fallback already owns `47891`). The loopback and LAN listeners keep sharing a port, as `lan_listeners.rs` requires, and mDNS advertises the actual port. Several instances on one host (channels, portable builds, `task dev`) take successive pairs, so the range holds about fifty. If the range is exhausted srv falls back to an OS-chosen port, and the indicator reports `needs-setup` because that port is not covered by the rule. Headless mode keeps `--web-port` / `--ws-port`.

**Windows rules.** A small mode of the signed host exe (proposed: `agentmux.exe --configure-lan-firewall`, so the UAC prompt names AgentMux) does, via `INetFwPolicy2`, in **one** elevated run:

1. Ensure **two** inbound Allow rules with fixed names, **not tied to any program**: `AgentMux LAN (TCP)` on local ports `47892-47991`, and `AgentMux LAN (UDP)` on `5353,47891` (mDNS and the broadcast fallback). Profiles **Private + Domain**, `RemoteAddresses = LocalSubnet`, edge traversal off. (Two rules because a port range needs a concrete protocol.)
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

Not yet known: whether macOS showed a Local Network permission prompt on starpower (the agent cannot see the screen; the operator must say), and how LAN was enabled there. **Area54** and **charlie** (a Linux VM on gamerlove, see section 6) were not on narko's LAN list at the time.

The same session also showed the failure this spec exists for: narko saw nothing while Area54 saw narko, until the Private profile was added by hand.

## 9. Delivery plan

One spec, separate PRs, in this order. Windows is the failing path and should not wait on macOS or Linux verification; each platform needs its own hardware to prove it.

| PR | Scope | Verified on |
|---|---|---|
| **A: core** | Fixed LAN port range with fallback; advertise the actual port; a firewall-status interface that reports coverage **per interface**; the new indicator states (`needs-setup`, `blocked`, `public-network`, `managed`). No OS-specific code. | unit tests; narko ↔ starpower |
| **B: Windows** | The elevated helper, the two port rules, Public-network consent, per-interface coverage gating of the LAN listeners and a decision on mDNS interface selection (4.2 caveat, settled by the measurement in section 6), installer step and uninstall cleanup. | a fresh Windows machine |
| **C: macOS** | Verify first; change code only if the Local Network prompt does not appear with our current discovery. | starpower |
| **D: Linux** | Detect an active `ufw` or `firewalld` and show the exact command. | charlie, once it is a bridged LAN member |

Separate from this spec: WAN delivery has no catch-up pull (a message that arrives before an agent subscribes waits for the next unrelated wake). Tracked on its own.
