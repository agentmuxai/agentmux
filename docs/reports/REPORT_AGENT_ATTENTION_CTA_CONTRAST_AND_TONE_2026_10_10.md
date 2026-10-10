# When an agent needs you: louder calls to action, and a tone for every one

**Status:** active — §1 (contrast) in PR #4604; §2 (the tone) in the PR after it. Not done yet: see "Left for later" at the end.
**Date:** 2026-10-10

## The requests

1. **Contrast.** When an agent needs the user's attention, its call to action should be styled with much higher contrast, the buttons above all: an attention-grabbing, fluorescent color, theme-aware (right in every dark and light theme).
2. **Tone.** While an agent waits on such a call to action, a tone should sound, like the one an `AskUserQuestion` plays.

Both came up while the operator was approving an agent's browser hand-off and npm sign-in. The banner was easy to miss, and it made no sound.

## Where agents ask the user to act today

| Call to action | Where | Component | Main button today | Tone | Flash | OS toast / taskbar |
|---|---|---|---|---|---|---|
| Browser hand-off ("needs you: …"), click approval, navigation ("wants to: …") | strip under the browser pane's nav bar | `view/browser/browser-view.tsx:239-307`, `browser-view.scss:136-180` | `Button tone="accent"` (outlined) on `--highlight-bg-color` | none | none | none |
| `AskUserQuestion` | docked at the bottom of the agent pane | `view/agent/components/AgentQuestionPanel.tsx:446-580`, `AgentQuestionPanel.scss` | `Button tone="accent"` "Submit answer"; accent border | **waiting tone** | none | yes (`input_waiting`, after 6 s), Windows taskbar flash |
| Tool permission ("Decision required") | bottom of the agent pane | `view/agent/components/AgentDecisionPanel.tsx:270-445`, `styles/_decision-panel.scss` | own buttons: Allow 22% success tint, Deny 18% error tint | none | none | none |
| SSH consent, add/remove host, end session, helper install, askpass | separate approval window | `view/ssh-approval/SshApprovalWindow.tsx:104-151`; srv `app_api/connections.rs` `ask_user` | `className="green solid"`, which renders as the outlined accent tone | none | none | none |
| Widget install by an agent | modal in the main window | `block/widget-install-requests.tsx` (`ConfirmModal`) | `tone="accent"` | none | none | none |
| Memory adoption / release | separate approval window | `view/memory-adoption-approval/MemoryAdoptionApprovalWindow.tsx` | `green solid` (outlined accent) | none | none | none |
| Saved-credential approval | separate window | `view/credential-approval/CredentialApprovalWindow.tsx:110-132` | `green solid` | none | none | none |
| Browser HTTP sign-in | modal in the pane | `view/browser/components/BrowserAuthModal.tsx` | `green solid` | none | none | none |
| Camera / microphone for a page | window modal | `window/pane-media-permission-prompt.tsx:128-141` | `ConfirmModal destructive`: Allow is the danger tone | none | none | none |
| Pane about to shut down | row in the agent pane | `view/agent/shutdown/ShutdownPendingBanner.tsx` | error-tinted "Keep running" | one-shot chime (at the start and at 5 s left) | none | yes (srv `pending_shutdown`) |

So the `AskUserQuestion` panel is the only call to action that plays the waiting tone. Every other place an agent waits on the user is silent, and the browser banner, the one the operator met, has the weakest styling of all: a 20% white tint and an outlined button.

## Why everything looks quiet

- **The button system never fills.** `element/ui/Button.tsx:24-28` says so: tones (`accent`, `neutral`, `danger`, `quiet`, in `ui/shared.tsx:11`) draw text and a 35% line in their color with a 6% tint (12% on hover), per `element/ui/_line.scss:18-51`. The old `<Button className="green solid">` maps to the same outlined accent tone (`element/button.tsx:28-34`), so no approval button is actually green or solid.
- **No theme has an attention color.**
  - The base tokens are in `app/theme.scss`: `--accent-color` (line 71), `--highlight-bg-color` (92), `--error-color`, `--warning-color`, `--success-color` (97-99) and `--info-color` (104, only in `:root`).
  - The named themes are in `app/themes/*.scss`: eight dark (midnight, high-contrast, monokai, nord, dracula, catppuccin, tokyo-night, gruvbox) and four light (light, catppuccin-latte, solarized-light, gruvbox-light).
  - Each theme overrides the accent and status colors, but none has a vivid or "needs you" color, so a call to action can only borrow the accent, which is also used for links, focus rings and selection.
- **The banner background is a plain highlight** (`--highlight-bg-color`, `rgba(255,255,255,0.2)` in the dark default), the same as hover states.

## 1. Contrast: a "needs you" style

**Tokens.** Add per-theme tokens for exactly this use, defined in `theme.scss` and every `themes/*.scss`:

- `--attention-color`: a fluorescent hue that no other UI uses, so the user learns that it means "an agent needs you". A vivid magenta (around `#ff2bd6`) or electric lime (around `#c6ff00`) both stand out against every current accent; magenta is the safer choice next to the many green and blue accents. Light themes need a deeper, still saturated shade of the same hue to stay readable.
- `--attention-text-color`: the text on a filled attention button, picked per theme for contrast.
- `--attention-bg-color`: a tinted background for the banner or panel around the call to action.

Each pair must meet WCAG AA (4.5:1 for button text, 3:1 for the button against its background) in every theme. A test should check the computed contrast of the tokens per theme, the way theme values are already checked elsewhere.

**A filled button.** Add an `attention` tone to the shared `Button` that fills: `--attention-color` background, `--attention-text-color` text. It's a deliberate exception to "never fills", and it's kept to calls to action only. Its style:

- a 2px outline on focus;
- an optional soft pulse of the glow while the agent waits, off under `prefers-reduced-motion`.

**Where it applies.** The main "yes, do it" action of each row in the table above. The decline actions (Cancel, Deny, Don't install) stay neutral. A decision with a destructive answer keeps `danger` on that answer, so the louder color never makes "Allow" on something dangerous look like the safe default; the camera prompt stays as it is for the same reason.

- **The browser attention banner** gets `--attention-bg-color` and a 3px attention border on the left. The icon and "needs you:" go in the attention color, the main button is filled, and the text is a size larger.
- **The `AskUserQuestion` and permission panels** get an attention border and header in place of accent and warning, and an attention-toned main button. Their risk pill stays error-colored.
- **The approval windows** (SSH, memory adoption, credential approval) and the widget install modal get the filled button. The `green solid` call sites move to `tone="attention"`, which also retires that old class there.

**Lint.** `npm run lint:theme-colors` only catches Tailwind palette classes in `.tsx`. The SCSS hex rule (stylelint `color-no-hex`, `npm run lint:scss`) isn't run in `ci-pr.yml`. New colors belong in the theme files only, as tokens; everything else uses `var(--attention-…)`.

## 2. Tone: one "waiting for you" sound for every call to action

**How the tone works now** (`app/notification/sound/`):

- **The sound.** The waiting tone is synthesized, not a file (`waiting-tone-player.ts:24-37`): a looping C5–E5–G5 sine arpeggio through a 1200 Hz lowpass, with fades. It uses the waiting volume `notify:sounds:waiting:volume` (0.25).
- **What starts it.** It plays only on the frontend `waiting-for-input` event, which `view/agent/hooks/useAgentQuestions.ts:103-160` fires when an `AskUserQuestion` tool node is pending. The hook also writes `term:awaiting_user`, the meta srv's `AgentState::Waiting` reads.
- **When it pauses or stops.** It pauses while its pane is focused and the window is in front (`notify:sounds:suppresswhenfocused`), and stops after 5 minutes (`WAITING_AUTO_STOP_MS`).
- **Settings.** It is governed by `notify:sound:agent.waiting.for.input` and the master switch, `notify:sounds:enabled`.

**The plan: make "waiting for you" a request, not a question.** Keep one registry of the open calls to action. Each entry has:

- a key, such as a block and request id;
- the pane it belongs to;
- what it is: question, permission, browser attention, SSH consent, widget install, memory adoption or credential.

The tone loops while the registry is non-empty for a pane, and stops when that pane's last request is answered or expires.

**Feeding it:**
- **Agent pane (frontend):**
  - `useAgentQuestions`, as today;
  - `useAgentDecisions`, for the tool permissions that are silent today;
  - also set `term:awaiting_user` for permission prompts, so `AgentState::Waiting`, the Swarm line and the LAN and viewer feeds see them too.
- **Browser pane (frontend):** the `browser:attention` block meta (`browser-model.ts:381-384`), present while srv waits on a hand-off, click approval or navigation.
- **Asked by srv:** the SSH consent and host questions in `app_api/connections.rs` `ask_user`, the widget install requests (`widgetrequests` event), and memory adoption. srv should report these through its notify router as the same `InputWaiting` kind `AskUserQuestion` already uses. Then they also get the OS toast and the Windows taskbar flash (`notify:os:*`, `notify:taskbar:attention`), even when the asking pane's tab isn't mounted. Today `browser_attention.rs` and `connections.rs` never call the router.

**Gaps to fix along the way:**
- **Tab switches.** The tone stops when its pane unmounts, so a tab switch silences a question that is still waiting. The registry should live per window, not per mounted pane, and stop only when the request ends.
- **Autoplay.** A request that arrives before the user's first click or key press in that window is silent, because the audio engine starts on the first gesture (`sound-service.ts:101-117`). Keep that browser rule, and rely on the OS toast and taskbar flash for it.
- **Flash.** The waiting tone has no flash: `startWaiting` never calls `emitActivityFlash`. Pulse the pane and window tabs with the tone, in the attention color.
- **Settings.** Rename the setting to say what it now covers ("An agent is waiting for you"), keeping the old key's value. Fix its out-of-date description in `schema/settings.json`. Consider a separate toggle for browser hand-offs, which the user is often looking at already.
- **Repeats.** A request that waits a long time could ring once more before the 5-minute auto-stop, as the shutdown banner does at 5 s left.

## Where to start

The browser attention banner: the lowest contrast, no sound, and the place the operator hit it.

1. The tokens, and the filled `attention` button tone.
2. The banner's styling.
3. Its `browser:attention` meta feeding the waiting tone.

Then the permission panel, which has the same gap. After that, the approval windows and srv's `InputWaiting` reports.

## As built (§2)

- **The registry.** `frontend/app/notification/waiting-for-you.ts`, per window. A pane waits while any of its sources does:
  - `question` (`useAgentQuestions`);
  - `permission` (`useAgentDecisions`);
  - `srv:<key>`, srv's announcements.
- **srv's announcements.** `backend/user_attention.rs` `Asking` publishes `userattention` from start to end, its `Drop` sending the end. It's used in three places:
  - `browser_attention::ask` (hand-offs, approvals and navigation);
  - `connections::ask_user` (SSH consent and other host questions);
  - the widget install route, for the verified agent's pane.
- **Which window.** An announcement names the windows showing its pane (`resolve_click_target`). Requests with no pane go under an `app:` key, play only while the window isn't in front, and send no OS notification.
- **Tab switches no longer silence it.** The agent hooks keep waiting through an unmount (a tab switch) and end only when the pane is gone.
- **The flash.** The tone's tab flash comes once per loop (`WAITING_LOOP_MS`), under the tool-tones flash setting.

## Left for later

- **Late windows.** A window opened while srv holds a request open doesn't hear it until the next one. srv keeps no list of open announcements.
- **Permissions in `AgentState::Waiting`.** A tool permission doesn't set `term:awaiting_user` yet, so it isn't in `AgentState::Waiting` or the Swarm's "Waiting for you" line.
- **Separate toggles.** No separate setting for browser hand-offs. No repeat chime before the 5-minute auto-stop.
