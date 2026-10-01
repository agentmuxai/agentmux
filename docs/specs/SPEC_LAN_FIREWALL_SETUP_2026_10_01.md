# SPEC: LAN that works on a fresh install, with no manual firewall steps

**Date:** 2026-10-01
**Status:** proposed — research and design; nothing implemented. Supersedes the "disable mDNS by default" stance of `windows-firewall-fix.md`.
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

- srv binds **loopback, OS-chosen ports** at startup and `LanListenerSupervisor` adds LAN listeners on the *same* ports when LAN is enabled (`bootstrap/network.rs`). No fixed port, so a port rule is impossible; a **program rule** is the natural fit (R9).
- mDNS uses the `mdns-sd` crate: raw sockets on UDP 5353, i.e. exactly the app-owned responder that needs its own UDP rule (R8).
- Windows ships three ways: **portable zip** (no installer), **Inno Setup per-user installer** (`packaging/windows/agentmux.iss`, no admin), and **MSIX**. A fresh install has no admin and no rules.
- LAN is **opt-in** (HostPopover toggle). `windows-firewall-fix.md` kept the prompt away by keeping mDNS off. That avoided the dialog but left LAN unusable until the owner hunted for a toggle, and gave no help when it broke.
- macOS already declares both Local Network keys (`scripts/package-macos.sh:386`). Whether raw-socket mDNS triggers the prompt is unverified (R10).
- The indicator has `off | idle | peers | error`. A firewall block lands in `idle`, the "healthy" state.

## 4. Design

### 4.1 One stable rule, owned by an elevated helper (Windows)

A small mode of the existing binary (proposed: `agentmux.exe --configure-lan-firewall`, so the UAC prompt names AgentMux) that does, via `INetFwPolicy2`, in one elevated run:

1. Ensure **one** inbound **Allow** rule, fixed `Name = "AgentMux LAN"`: program = the **current** `agentmux-srv` exe path, protocol Any, profiles **Private + Domain**, `RemoteAddresses = LocalSubnet`, edge traversal off. Update the program path in place if it already exists (R4, R5).
2. **Delete any Block rules** whose program is an AgentMux binary (left behind by a cancelled prompt, R2).
3. **Prune** AgentMux rules whose program no longer exists (the 606), keeping ours.
4. Exit codes: `0` ok, `1223`-style cancel, `2` policy-managed (R11), `3` other. Idempotent; running it twice changes nothing.

Public is **not** in the rule. Opening a listener to a Public network is the wrong default; see 4.4.

### 4.2 Order: rule first, then listeners (R1)

Enabling LAN becomes:

1. Read the firewall state **without admin** (`INetFwPolicy2` read, `INetworkListManager` for profiles).
2. If the rule for the current srv exe is present and enabled and no block rule exists: bind the LAN listeners and start mDNS. **No OS dialog can appear**, because a matching allow rule exists.
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

### 4.4 Network profile is a message, not a rule

If the adapter is Public, do **not** quietly open Public. Detect it (no admin) and explain it (4.3). This is the case that bit narko when the prompt's default profile did not match the network later in use.

### 4.5 Installers

- **Inno per-user installer:** an optional task, "Allow LAN access (asks for administrator permission)", that runs the helper with `runas`. Unticked or declined, the first LAN enable does it (4.2). Uninstall runs the helper's remove mode.
- **Portable / dev builds:** no installer, so 4.2 is the whole mechanism. Path changes per build are handled by 4.1 (update in place, prune old).
- **MSIX:** verify separately whether the package identity changes the prompt behaviour; not assumed here.

### 4.6 Hardening, phase 2 (separate PRs)

- **Windows mDNS through the OS** (`DnsServiceRegister` / `DnsServiceBrowse`): inbound mDNS goes through the DNS Client service's built-in rule, so discovery would not depend on our UDP rule at all (R8). Our TCP connection to a peer still needs 4.1. Must be verified on a clean machine before relying on it; today it is an inference.
- **macOS:** confirm on a clean macOS 15 machine that `mdns-sd` raw sockets raise the Local Network prompt; if not, discover through the system Bonjour API (R10).
- **Linux:** detect an active `ufw` or `firewalld` and show the exact one-line command; do not silently modify it.

## 5. Acceptance (a fresh machine is the test)

On a clean Windows 11 VM with no AgentMux rules, per-user install, standard (non-admin) login that can approve UAC:

1. Enable LAN → one explanatory sentence, **one** UAC prompt, **no** "Windows Security Alert". Within 60 s the peer list fills **in both directions**, and a LAN jekt arrives `lan-verified`.
2. Decline UAC → AgentMux keeps working loopback-only; the indicator reads `needs-setup`; no prompt on the next start.
3. Cancel the *Windows* dialog in a build that predates this (block rule present) → helper removes the block; LAN works.
4. Network marked Public → indicator reads `public-network` with the Settings link; marking it Private makes LAN work with no further action.
5. A second build at a new path → the rule follows the new path; the old rule is gone; the rule count does not grow.
6. Uninstall → no AgentMux rule remains.
7. Two machines on different Windows versions, one per direction, exchange a plain and a keyword jekt over LAN.

## 6. Open questions

- Which binary does the UAC prompt show? It should be the signed host exe, not the sidecar, so the publisher reads AgentMux.
- Is `RemoteAddresses = LocalSubnet` enough on this fleet, or do some hosts (VPN, WSL, Hyper-V adapters, as on narko) need an explicit range? Narko lists four LAN listeners, three on virtual adapters.
- IPv6 link-local: `Area54.local` resolved to an `fe80::` address. Confirm the rule and the listeners cover it.
- The product currently treats a failed LAN start as an error only when the mDNS daemon itself fails; confirm what `lan_discovery_error` should add.

## 7. Immediate unblocker (not the fix)

Until 4.1 exists, add the Private profile to the existing rule (Windows Security → Firewall → Allow an app → `agentmux-srv-…` → tick Private), or run, elevated:

```powershell
netsh advfirewall firewall add rule name="AgentMux LAN" dir=in action=allow protocol=any profile=private,domain remoteip=localsubnet program="<full path to agentmux-srv.exe>"
```

It has to be repeated for every new build path, which is exactly why 4.1 exists.
