# RCA: a limited browser pane goes blank when sign-in leaves its sites during a hand-off

**Date:** 2026-10-16
**Status:** analysis — fixed by the PR that adds this report: §6.1–6.3 and §7 are built as written; what was left out is in §8.
**Author:** Naki #2 (naki-naki-2)
**Trigger:** Repo owner, 2026-10-16: an agent opened a browser pane limited to `github.com` and handed it over to sign in. Clicking "Continue with Google" turned the pane black; Back worked. "This used to work fine." The owner asked that a stopped navigation never end in a blank pane, and whether the code needs to be made more DRY.
**Code:** `main` @ `2e8ab7fa5`; the build that showed it is `0.59.28+g560b0f909`. None of the files below changed between the two.
**Related:** `SPEC_BROWSER_PANE_ALLOWED_ORIGINS_2026_10_09.md` (the site list, #4544, #4562), `SPEC_AGENT_DRIVEN_BROWSER_PANES_2026_10_07.md` §5.2 (hand-off, #4460).

---

## 1. Summary

- **What the person saw:** GitHub's "Redirecting to Google" page, which is empty and dark, left in place because the redirect after it was stopped. No question, no message.
- **Why:** a hand-off and the site-list question use the same per-pane attention slot. While the pane is handed to the person, `waiting_on_user(pane)` is set, so srv answers the host's "a navigation left the list" report with `asked: false` and asks nobody (`crates/srv/src/server/browser_allowlist.rs:238-240`). The host had already cancelled the navigation in `on_before_browse`. Nothing loads, nothing is shown, and nothing is logged.
- **Regression:** yes. #4544 (2026-10-10) added the site list on top of the hand-off from #4460 (2026-10-08). Before it, agent panes had no list and sign-in redirects went through. Panes opened without `allowed_origins` still behave as before.
- **The deeper fault:** the host cancels first and leaves the outcome to srv. srv then has five ways to end the attempt, and only two of them load something or show something. A sign-in during a hand-off is the most common path into one of the other three, but not the only one (§4).
- **Recommended fix (§6):**
  1. Ask about off-list navigations during a hand-off too.
  2. Make one rule hold everywhere: every navigation the host stops ends in a load, a question, or a visible notice in the pane.
  3. Put that rule in one place, which answers the DRY question: yes, in a targeted way (§7).

## 2. Symptom

1. An agent calls `Browser` `open` with `url` on github.com and `allowed_origins: ["github.com"]`, then `handoff` ("sign in").
2. The person clicks "Continue with Google" on GitHub's sign-in page.
3. The pane shows an empty dark page titled "Redirecting to Google". It stays like that. Clicking again repeats it.
4. Back returns to GitHub's sign-in page.

## 3. What the logs show

Host log, pane `c18ea5c8` (OAuth parameters trimmed):

```
20:37:31.629 pane-trace  load-end url=https://github.com/sessions/social/google/initiate?...
20:37:31.630 display     emit-title-change title="Redirecting to Google"
20:37:31.641 allowed_origins  browser pane: stopped a navigation off its allowed sites
                              url=https://accounts.google.com/o/oauth2/v2/auth?...
20:37:31.645 pane-trace  load-error url=https://accounts.google.com/o/oauth2/v2/auth?...
```

srv log for the same pane: `browser open (agent-owned)` at 20:37:14, `browser snapshot`, `browser hand-off` at 20:37:23, and then nothing. No `[browser-allowlist] asking the user`, no `not allowed`, no `couldn't ask`. The host logged no `couldn't tell srv about a navigation`, so the report reached srv and srv answered `asked: false`. The second attempt at 20:37:57 is identical.

## 4. Root cause

### 4.1 The host cancels, then reports

`on_before_browse` (`crates/cef/src/client/lifecycle.rs:1168`) calls `stop_off_list_navigation` (`crates/cef/src/client/allowed_origins.rs:123`). For an off-list main-frame URL it returns true, which cancels the navigation, and `report_off_list` posts `{pane, target, url, kind}` to `POST /api/v1/host/browser_navigation` on its own thread. The design is deliberate: CEF's callback runs on the UI thread and can't wait for a person (spec §6).

On a redirect hop, "cancelled" means the frame stays on the document that started the redirect. Here that is GitHub's `/sessions/social/google/initiate`, an interstitial with no visible content. The cancel can't leave the person anywhere useful. Only the next step can.

### 4.2 srv refuses to ask while a hand-off is up, silently

`handle_host_browser_navigation` (`crates/srv/src/server/browser_allowlist.rs:203`) ends the report in one of five ways:

| Branch | Lines | What the person gets |
|---|---|---|
| Pane no longer limited | 218-226 | replay: the page loads |
| URL is on the list after all | 227-234 | replay: the page loads |
| Not a web address | 235-237 | **nothing**, no log |
| `waiting_on_user(pane)` is set | 238-240 | **nothing**, no log |
| Ask in a background task | 245-280 | a question. Block, a timeout and `couldn't ask` (debug only) end in **nothing** |

`waiting_on_user` (`crates/srv/src/server/browser_attention.rs:83`) is true for any pending request on the pane, and the hand-off is one: `Kind::Handoff`, parked by `ui_handlers.rs:1193` for up to 15 minutes. `ask()` enforces the same rule a second time (`browser_attention.rs:193-195`: "this pane is already waiting for the user").

The spec's "one question at a time per pane" (§4) was written for two questions colliding: a second off-list attempt while an Allow/Block question is up is dropped, and the banner already explains why. A hand-off is not that case. Its banner says "needs you: sign in", says nothing about the stopped navigation, and the person is the one driving.

### 4.3 Why it reads as "used to work"

- 2026-10-08, #4460: hand-off ships. Agent panes have no site list, so a sign-in can go anywhere.
- 2026-10-10, #4544 and #4562: `allowed_origins` ships. Every off-list main-frame navigation is cancelled on the host and becomes a question in srv, except during a hand-off.
- The `Browser` tool tells agents to "include every site a sign-in on the way needs". An agent can't know in advance which identity provider a person will choose (Google, Microsoft, SSO, passkey pages), so the list will be missing it whenever the person picks one the agent didn't predict.

### 4.4 The general fault: cancel-then-hope

The host's cancel is final unless srv replays. Every path that neither replays nor shows a question leaves the pane wherever the cancel left it:

| Path | Where |
|---|---|
| Hand-off or approval up | srv §4.2 |
| Not a web address | srv §4.2 |
| Block, no answer in 10 min, Take over | srv background task |
| `ask()` fails (banner broadcast error, race on the slot) | srv background task, debug log |
| srv unreachable or not JSON | host `report_off_list`, warn |
| Report thread couldn't start | host `report_off_list`, warn |

When the stopped navigation was a link click, "where the cancel left it" is the page the person was on, and the result is merely confusing. When it was a redirect hop, as in every sign-in flow, the page is an interstitial, and the pane is blank.

## 5. Impact

- Any agent pane opened with `allowed_origins` and handed over for a sign-in through another site fails, with no explanation. That is the main reason an agent hands a pane over.
- The person can't fix it from the pane. Allow never appears, and Take over isn't obvious: it ends the agent's hold, which is the only thing that lifts the list.
- Not a security problem: the guard fails closed. The agent can't go anywhere it shouldn't.

## 6. Fix

### 6.1 Ask during a hand-off (the bug)

A stopped navigation during a hand-off or an approval should still be asked about. Recommended shape:

- `browser_attention` keeps **one Navigation question per pane, in addition to** a Handoff or Approval: a two-level slot instead of one. The banner shows the Navigation question while it is up, and the hand-off banner returns after it is answered. The hand-off's own wait (its `Waiting`, its timeout, the agent's tools staying locked) is untouched.
- `handle_host_browser_navigation` drops only when a **Navigation** question is already up (`asking(Kind::Navigation)` contains the pane), not when any request is.
- Allow, as today: the origin joins the list, and the stopped address loads (`replay_navigation`). In this case that takes the person straight to Google's sign-in and the flow continues.

**Not recommended:** letting off-list navigations through without asking while a hand-off is up. It's simpler, but the guard exists because a page can steer the pane. A redirect isn't the person's choice, and when the hand-off ends the agent gets the pane back on a site it was never limited to.

### 6.2 Never end a stopped navigation in a blank pane (robustness)

Make one rule hold everywhere: **every navigation the host stops ends in exactly one of: it loads, the person is asked, or the pane says why it stopped.** Concretely:

- **A notice kind for the banner** (`Kind::Notice`, non-blocking: it doesn't lock the agent's tools and doesn't time out). Example: "Stopped: this pane is limited to github.com, and the page tried to go to accounts.google.com." It offers **Back**, and **Allow** whenever srv can still add the origin. It replaces the silent branches in §4.2 and §4.4: not a web address, Block and timeout (the person sees what they declined), `couldn't ask`.
- **Host fallback for when srv can't be reached:** if the report fails or srv answers that it neither asked nor replayed, the host shows the same notice locally. It can be a native message in the pane's nav strip, so it doesn't depend on srv writing `browser:attention`.
- **Redirect hops:** when the stopped URL was a redirect hop off an interstitial, the notice's Back should go to the last page the person actually used. Here plain Back did reach GitHub's sign-in page, but an interstitial that redirects again on load would loop.

### 6.3 Logging

Every outcome logs at `info`, naming the branch: `asked`, `replayed`, `refused: <reason>`, `notice shown`. Today two branches return with no log at all and `couldn't ask` is `debug`, which is why §3 had to be read from what was missing.

### 6.4 Tests

- srv: with a Handoff parked on a pane, an off-list report returns `asked: true` and the banner shows the Navigation question. When it's answered, the Handoff banner is back and the hand-off's wait is still pending.
- srv: a report while a Navigation question is up returns `asked: false` with a reason, and no second banner appears.
- srv: each refusal branch produces a Notice banner.
- Host: `report_off_list` with an unreachable srv results in the local notice.
- e2e (isolated dev build): open a pane limited to a test origin, hand it off, follow a redirect to a second test origin, Allow, and land on the second origin.

## 7. Does the code need to be more DRY?

Yes, but in a targeted way. Duplication isn't the root cause. The missing invariant in §6.2 is, and that invariant can't be enforced while the outcome of a stopped navigation is decided in six places. The consolidations that matter:

1. **One owner for the outcome of a stopped navigation.** Today it is split across the host's cancel and report thread, srv's five handler branches and srv's background task. Each new branch had to remember to replay or show something, and four didn't. Proposed: one srv function, `settle_stopped_navigation(pane, target, url, kind) -> Settled { Replayed | Asking | Notice(reason) }`, that every branch returns through, with `Notice` rendered in one place. The host treats anything other than `Replayed` or `Asking` the same way: the notice.
2. **"One request per pane" is checked twice, silently, with different consequences.** The handler's `waiting_on_user` pre-check (`browser_allowlist.rs:238`) and `ask()`'s own check (`browser_attention.rs:193`). Keep the rule in `browser_attention` only, with a typed error (`Busy { kind }`) the caller must handle, so a busy slot can't turn into silence.
3. **"Push the list, then replay if it was a navigation"** is written out three times (`browser_allowlist.rs:221-224`, `229-232`, `270-273`). Make it one helper.
4. **The banner's presentation branches on `kind` in about eight inline ternaries** (`frontend/app/view/browser/browser-view.tsx:252-312`): label, verb, tone, density, button text, which answer to send. Adding `Notice` (§6.2) would make that worse. Use one `kind → {label, tone, primary, secondary, answer}` table in the browser model.

Not worth merging: the host's `stop_off_list_navigation` and `stop_off_list_popup` (`allowed_origins.rs:123-144`). They share `off_list` already and differ in what they resolve and report.

## 8. What the fix built, and what it left out

Built (2026-10-17):
- **§6.1:** `browser_attention` has the two-level slot. A pane may have one Navigation question or Notice over its hand-off, approval, auto-approve question or page dialog. Each request keeps its banner, so the one underneath comes back when the question is answered. `handle_host_browser_navigation` drops a report only while another Navigation question is up, and logs why.
- **§6.2:** `Kind::Notice`, for not a web address, Block, a timeout, and a question that couldn't be shown. It offers Back, Allow when the site can still be added, and Dismiss. It doesn't pause the agent's tools or time out. The host shows its own notice (an event to the pane, `browser-pane-notice`) when srv can't be reached or doesn't settle the report.
- **§6.3:** every outcome is logged at `info`, with the branch name (`replayed`, `asking`, `busy`, `notice`, `unshown`) and the reason.
- **§7.1–7.3:** `settle_stopped_navigation` decides the outcome in one place and answers the host with it (`settled`). `push_then_replay` is the one "push the list, then replay" helper. `ask`/`raise` return a typed `AskError::Busy { kind }`. **§7.4:** `notice` is a row in `attention-kinds.ts`.

Left out:
- **§6.2, redirect-hop Back:** Back is the pane's own history Back. Going back to "the last page the person actually used" would need the host to tell interstitials from real pages, and no reliable signal for that was found.
- **§6.4, the e2e check** in an isolated dev build: not run. The srv route test covers the same flow against a stand-in host. The host test covers the decision to show the local notice, not the CEF event itself.
