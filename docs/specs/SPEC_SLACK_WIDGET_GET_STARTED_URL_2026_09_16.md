# SPEC: Slack widget should open Slack's get-started/create-workspace page, not the generic sign-in page

**Author:** Vmer
**Date:** 2026-09-16
**Status:** implemented — #3269

---

## 1. Problem

The `Slack` messenger widget (`defwidget@slack` in `agentmux-srv/src/config/widgets.json`) is a plain CEF browser pane pointed at a static URL — same architecture as its four siblings (Discord/Telegram/WhatsApp/Teams), confirmed in `docs/status/STATUS_MESSENGER_APP_INTEGRATIONS_2026_07_07.md` §2: *"a CEF browser widget pointed at the platform's real web app, address bar hidden... no custom chat UI exists for any of them; the messaging app IS the UI."* No connection-state logic exists for any of these widgets — the URL is static config, not derived at runtime.

Today that URL is `https://app.slack.com/` — Slack's general sign-in/workspace-picker page. For a user who hasn't already got a Slack session/workspace, this page is a dead end: it asks for a workspace URL or email with no path forward to actually creating one. The widget's own description ("Slack — real interface, agent-connected") implies a user reaching for this widget for the first time is trying to get set up, not sign into an account that may not exist yet.

## 2. Fix

Change the one config value. `https://slack.com/get-started?entry_point=home_page#/createnew` is Slack's own dedicated "create a new workspace" onboarding flow — the correct landing page for a first-time user, and still a normal, fully-functional page for a returning one (Slack's own UI offers sign-in from there too).

```diff
     "defwidget@slack": {
         "display:order": 11,
         "display:pinned": false,
         "display:hidden": true,
         "icon": "brands@slack",
         "color": "#4a154b",
         "label": "Slack",
         "description": "Slack — real interface, agent-connected",
         "blockdef": {
             "meta": {
                 "view": "browser",
-                "url": "https://app.slack.com/",
+                "url": "https://slack.com/get-started?entry_point=home_page#/createnew",
                 "browser:show_controls": false
             }
         }
     },
```

No other widget in this file changes — Discord/Telegram/WhatsApp/Teams each point at their own platform's general app entry point by design, and none of them were reported as having this problem.

## 3. Non-goals

- No dynamic "already connected, skip onboarding" logic. Every other messenger widget is a static URL with no connection-state awareness; adding that just for Slack would be a real architecture change (needs to know whether the embedded webview already has a live Slack session — not currently tracked anywhere) for a problem the fix above already solves in practice, since Slack's own get-started page still offers sign-in.
- No change to the Slack **bridge** (`agentmux-srv/src/messaging/slack/`, the agent-facing Socket Mode/Web API integration) — this is purely the human-facing webview pane's landing URL, a completely separate code path per the status doc's own two-layer (pane + background bridge) architecture.

## 4. Verification

- `agentmux-srv/src/config/widgets.json` is plain JSON, parsed at startup — validated it's still well-formed JSON after the edit (`jq . agentmux-srv/src/config/widgets.json`).
- Manual verification (opening the widget in a live `task dev` instance and confirming the new page loads) was not performed in the environment this was prepared in — no GUI available. The change is a single string-literal URL in a static config file with no other logic touching it, so this is the practical extent of pre-merge verification available here.
