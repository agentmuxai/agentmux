# SPEC: An agent's browser pane can be limited to the sites it was opened for

**Status:** implemented — PR #4544: `OpenBrowser({allowed_origins})`, the host stopping navigations and popups off the list, and the Allow / Block banner.
**Author:** AgentX@narko, 2026-10-09, at the operator's request
**Builds on:** `SPEC_AGENT_DRIVEN_BROWSER_PANES_2026_10_07.md` §8.1 (the allowlist, left for B3b),
`SPEC_BROWSER_PANE_NATIVE_POPUPS_AGENT_DRIVEN_2026_10_08.md` §8.6 (it covers popups too).

## 1. Summary

`OpenBrowser({url, allowed_origins})` opens a pane that stays on the listed sites. When the
page tries to go anywhere else (a link, a form, a script, a redirect, a popup), the navigation
doesn't happen, and the pane asks the person: **Allow** or **Block**. Allow adds that site to
the pane's list and goes there; Block leaves the page where it is. The agent never answers.

The list guards against the page steering the agent: a page that tells an agent to "continue
at evil.example" can't take the pane there without the person agreeing. Without
`allowed_origins` nothing changes, since sign-in flows cross sites (§8.1 of the 2026-10-07 spec).

## 2. The list

Each entry is one of:

| Entry | Matches |
|---|---|
| `example.com` | `https://example.com` |
| `*.example.com` | `https://example.com` and every subdomain of it, over https |
| `https://example.com:8443` | that origin exactly |
| `http://localhost:3000` | that origin exactly; `http` only when written out |

A path, query or fragment in an entry is ignored; a malformed entry fails the `OpenBrowser`
call, naming it. Up to 32 entries. The pane's starting `url` must be on the list, or the call
fails. Matching is on the target's origin: scheme, host (in its ASCII form) and port.

## 3. What is checked

- **Main-frame navigations** of the pane: link clicks, form submissions, script navigations,
  back and forward, and each redirect hop.
- **Its popups:** the popup windows it opens and the popup panes it opens, and theirs. A chain of
  popups shares the list of the pane it started from.
- **Not** frames inside the page, and not the page's own requests (images, `fetch`). The list
  limits where the pane goes, not what the page loads: it is not a network sandbox.
- **Not checked:** `about:` pages. Non-web schemes are already refused for every pane.

An agent's own `BrowserNavigate` to a site off the list fails at once, with no question to the
person: the agent can open another pane for it, or ask the person in chat.

## 4. Asking

- **Where:** the pane's banner, like an approval (2026-10-07 spec §5.4): "This pane is limited
  to the sites it was opened for. The page wants to go to **origin**." For a popup window, the
  banner is on its opener and names the window (native-popups spec §7).
- **Allow:** the target's origin joins the list for the rest of the pane's life, and the pane
  (or popup window) loads the address. A form's data is not sent again: Allow loads the address
  as a plain page load. A popup is not reopened: the person or agent repeats the click, which
  now succeeds.
- **Block, no answer in 10 minutes, Take over, or closing the pane:** nothing loads.
- **One question at a time** per pane, as for approvals: while it waits, the agent's tools on that
  pane say so, and another off-list attempt is dropped without a second question.

## 5. Who owns the list

The list belongs to the agent's hold on the pane:

- Only `OpenBrowser` sets it. It lives in srv, and on the pane as `browser:allowed_origins`, a
  srv-only meta key: clients can't write it, and it is stripped from new, restored and
  layout-opened panes, like `browser:owner_agent`.
- **Take over** ends the hold, and the pane's list with it: the person browses freely there.
- Closing the pane drops its list too.
- The other panes of its chain keep the list, even when the pane that left was the one
  `OpenBrowser` opened: a popup pane can outlive its opener and still be the agent's, and must not
  lose its limit with it. The list goes when the last pane of its chain does.

## 6. How it works

- **Rule** (`agentmux_common::allowed_origins`): `parse` an entry, `allows(list, url)`. Shared
  by srv (validation, `BrowserNavigate`, the re-check of a request) and the host (enforcement).
- **srv** (`browser_allowlist.rs`): one list per chain, keyed by the root pane, with each popup
  pane mapped to its root. It pushes `{pane: list}` for every member to the host with the
  owned-pane set (`browser_host_sync`), and pushes at once, before the pane opens, when a
  popup pane joins a chain, so its first navigation is already checked.
- **Host:** `on_before_browse` runs synchronously and can't wait for an answer. For a main
  frame whose governing pane (the pane, or a popup window's opener) has a list and a target off
  it, it cancels the navigation and posts `/api/v1/host/browser_navigation` (host token) on a
  worker thread. `on_before_popup` does the same for a popup to a site off the list.
- **srv** re-checks the request against its own list, asks (`browser_attention`, kind
  `navigation`), and on Allow adds the origin, pushes the list to the host, then has the host
  load the address in the pane or popup window.
- **Frontend:** the banner's `navigation` kind, with Allow and Block; a "Limited to N sites" note
  in the pane's header while the list is set.

## 7. Tests

- **Rule:** entry forms, malformed entries, ports, `http` only when written, `*.` and the bare
  domain, internationalised hosts, `blob:` URLs, `about:`.
- **srv:** `OpenBrowser` validation (malformed entry, starting `url` off the list); the key is
  srv-only and stripped; Take over and close drop the list; a popup pane joins its root's chain;
  Allow adds the origin; a request for a pane without a list, or an origin on it, is refused.
- **Live** (`task dev`): a pane limited to one test site; a link to a second site is stopped and
  asked; Block keeps the page; Allow loads it and later links there pass; a popup window's
  navigation off the list asks on its opener; `BrowserNavigate` off the list fails.
