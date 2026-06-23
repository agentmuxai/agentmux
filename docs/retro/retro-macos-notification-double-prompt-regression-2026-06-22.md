# Retro: macOS dual notification toast — regression in 0.48.1

- **Date:** 2026-06-22
- **Severity:** Medium (UX regression; prompts for a capability we never use)
- **Status:** Resolved — PR #1713 removes `AgentMux Helper (Alerts).app` from the bundle (option A)
- **Scope:** macOS only — `AgentMux Helper (Alerts).app` notification registration

---

## 0. TL;DR

PR **#1659** (0.47.3) added `--disable-notifications` to `on_before_command_line_processing`
and was verified to give **0 prompts** (0 entries in `com.apple.ncprefs` after 5+ hours of
runtime). The fix is still in the code. Yet on first launch of 0.48.1 the user sees
**2 notification toasts** again. The prompt text is the standard macOS
"Notifications may include alerts, sounds, and icon badges" permission dialog.

`--disable-notifications` prevents JavaScript's `Notification.requestPermission()` — it
suppresses the JS-to-OS call path. What is now triggering the OS-level prompt is either
a changed macOS 26 Tahoe behaviour, a changed CEF native notification code path, or a
per-version-bundle-ID trigger that `--disable-notifications` no longer fully silences.

---

## 1. Prior history

### First report & investigation (0.47.2 / early 0.47.x)

Two macOS notification toasts appeared on launch. Root cause: **Chromium dual-bundle
registration**. Every Chromium-based app registers two notification sources:

1. The **main bundle** — `ai.agentmux.stable.<ver>`
2. The **AlertNotificationService helper** — `ai.agentmux.stable.<ver>.helper.alerts`
   (`AgentMux Helper (Alerts).app`)

Each calls `UNUserNotificationCenter requestAuthorization` independently. Per-version
bundle IDs (PR #1646) meant every upgrade was treated as a new app → 2 fresh prompts
per install.

### Fix: PR #1651

Added an icon to `AgentMux Helper (Alerts).app` so at least the prompt showed a proper
icon. Did not address the count.

### Fix: PR #1659 (0.47.3) — believed resolved

Added `cmd.append_switch(Some(&CefString::from("disable-notifications")))` in
`agentmux-cef/src/app.rs::on_before_command_line_processing`. Unconditional — applies
to every process type.

**Verification (2026-06-22, 0.47.4 session):**
```
defaults read com.apple.ncprefs apps | grep agentmux  → (no output)
```
Zero AgentMux entries in `ncprefs` after 5+ hours running from `/Applications`.
Decision: keep 0 prompts. Closed.

---

## 2. Regression: 0.48.1 shows 2 prompts

User reports 2 notification permission toasts on first launch of 0.48.1. The flag
`--disable-notifications` is **still present** in `agentmux-cef/src/app.rs:864`.
No commit between #1659 and 0.48.1 touches that line.

Immediate state at time of writing:
- `defaults read com.apple.ncprefs apps | grep agentmux` → 0 entries
- App has not been launched yet from the 0.48.1 DMG (toasts were seen during install/first-run)

---

## 3. The two bundles in 0.48.1

From `build/AgentMux.app/Contents/Frameworks/AgentMux Helper (Alerts).app/Contents/Info.plist`:

```xml
<key>CFBundleIdentifier</key><string>ai.agentmux.stable.0.48.1.helper.alerts</string>
<key>LSUIElement</key><true/>
```

No `NSUserNotificationsUsageDescription`, no explicit notification entitlements, no XPC
service declaration. It is a plain `APPL` bundle that CEF spawns as its notification
delivery helper.

---

## 4. Churn history

| PR | Change | Outcome |
|---|---|---|
| #1646 | Per-version bundle IDs | Side-effect: 2 prompts reset every upgrade |
| #1651 | Icon on Alerts helper | Cosmetic only — still 2 prompts |
| #1659 | `--disable-notifications` switch | Verified 0 prompts in 0.47.3/0.47.4 |
| 0.48.1 | Multiple unrelated changes | 2 prompts return |

Total: 4 PRs touching this (including this one).

---

## 5. Hypotheses (ranked by likelihood)

### H1 — macOS 26 Tahoe changed when/how the helper is discovered (most likely)

macOS 26 may now **proactively prompt** for notification permission when it discovers a
new app bundle containing a recognisable notification helper (`*.helper.alerts` / an app
in `Contents/Frameworks/` that matches Chromium's helper naming pattern). Earlier macOS
versions waited for the app to call `requestAuthorization`; Tahoe may scan on first
launch and prompt independently of whether the app requests permission.

This would explain why `--disable-notifications` (which only silences the CEF → OS
request path) no longer helps: macOS is prompting based on bundle structure, not
application request.

**Evidence for:** The fix was verified working on the SAME machine (macOS 26 Tahoe) in
the 0.47.3/0.47.4 session. Something else must differ between those runs and the 0.48.1
first install. First-install detection is the most likely macOS-level trigger.

**Evidence against:** Need to confirm by checking ncprefs immediately after first launch.

### H2 — CEF version update changed AlertNotificationService launch behaviour

If CEF was updated between 0.47.x and 0.48.x, the notification manager might now start
the Alerts helper earlier or via a different code path that runs before
`on_before_command_line_processing` can suppress it.

**Investigate:** Check CEF version in `Cargo.lock` — `cef-dll-sys` version — and compare
with the version in 0.47.3.

### H3 — Per-version bundle ID re-triggers macOS even for existing permissions

If the user had previously denied or allowed notifications for `ai.agentmux.stable.0.47.x`,
upgrading to `0.48.1` (new bundle ID) means a new, unknown app from macOS's perspective.
`--disable-notifications` should still prevent the request, but macOS 26 may have changed
the first-launch discovery flow (see H1).

### H4 — `--disable-notifications` suppresses JS but not the native XPC path

`--disable-notifications` maps to `switches::kDisableNotifications` in Chromium, which
disables `NotificationPermissionContext` and the web-notification service. The
`AlertNotificationService` helper in newer Chromium may register via a **native XPC
service** path that bypasses this switch. The `on_before_command_line_processing`
approach only modifies the Chromium command-line — it does not prevent the OS from
launching the helper as an XPC service registered in the main bundle's `Info.plist`.

---

## 6. Investigation needed (before fixing)

1. **Reproduce + capture ncprefs at the moment of prompt:**
   ```bash
   # On first launch of 0.48.1 — run immediately when toasts appear:
   defaults read com.apple.ncprefs apps | grep -A5 agentmux
   ```
   Are 2 entries appearing (even transiently) or is the prompt appearing before registration?

2. **Check whether `on_before_command_line_processing` fires for the Alerts helper process:**
   Add a log line gated on `process_type == Some("alert-notification-service")` or
   similar — verify the switch is actually being applied to the right process.

3. **Check CEF version delta:**
   ```bash
   grep "cef-dll-sys" Cargo.lock | head -3
   ```
   Did CEF update between 0.47.3 and 0.48.1?

4. **Test removing the Alerts helper entirely:**
   In `scripts/package-macos.sh`, remove `AgentMux Helper (Alerts).app` from the bundle
   before signing. If the 2 prompts disappear, the helper is being launched by the OS
   independently of `--disable-notifications`. This is the strongest evidence for H1/H4.

5. **Test `--disable-features=NotificationsViaHelperApp`:**
   Already in the memory as the "exactly 1 prompt" option. But if macOS is prompting
   independently (H1), this won't help. Test to narrow down whether CEF or macOS is the
   initiator.

---

## 7. Fix options

| Option | Expected result | Risk |
|---|---|---|
| **A) Remove `AgentMux Helper (Alerts).app` from bundle** | 0 prompts (no helper = nothing for macOS to discover) | CEF may complain at startup if it looks for the helper; test needed |
| **B) Add `--disable-features=NotificationsViaHelperApp`** | 0 or 1 prompt depending on whether macOS or CEF initiates | Doesn't help if macOS scans independently (H1) |
| **C) Rename / reclassify the Alerts helper** | If macOS uses naming convention to detect it, renaming breaks detection | CEF hardcodes the helper name; requires source patch |
| **D) Strip notification entitlements from the helper Info.plist** | May prevent macOS from treating it as a notification-capable bundle | Risk of Chromium crash if entitlements are required |
| **E) Keep `--disable-notifications`; accept 1st-launch prompts** | Users see 2 prompts once per version; can dismiss; no functional regression | Goes against "0 prompts" decision |

**Recommended first action:** Try option A (remove `AgentMux Helper (Alerts).app`
from the package script). If CEF starts without error, this is the cleanest fix. Add a
comment explaining why the helper is excluded.

---

## 8. Durable guard

Once fixed, add to `scripts/package-macos.sh` verify step:

```bash
# No AgentMux Helper (Alerts).app should be in the signed bundle
if [ -d "$APP/Contents/Frameworks/AgentMux Helper (Alerts).app" ]; then
    echo "❌ AlertNotificationService helper present — will trigger macOS notification prompts" >&2
    exit 1
fi
```

And re-add the `defaults read com.apple.ncprefs` check to the post-install verification
plan so a regression is caught immediately on first launch.

---

## 9. What NOT to do

- **Do not just remove `--disable-notifications`.** The switch also suppresses the JS
  `Notification` API. Without it, any frontend code (or a future library) that calls
  `Notification.requestPermission()` would show a prompt.
- **Do not paper over with per-version ncprefs pre-seeding.** Manipulating ncprefs
  directly is fragile, undocumented, and breaks across macOS updates.
- **Do not add `NSUserNotificationsUsageDescription` to the main bundle.** That would
  cause macOS to show the permission dialog UI (it's an opt-in intent declaration).
