# Spike: Chrome's address bar autocomplete in browser panes, through a CEF patch

**Date:** 2026-10-12
**Status:** analysis — spike done; the patch works. Not productized.
**Author:** Agent1
**Question (repo owner):** can a browser pane's address bar get Chrome's own completion (history, typed URLs, search suggestions), and is a CEF patch really the way?
**Code:** `agentmuxai/cef` branch `agentmuxai/8037` @ `6fba2047e` (CEF 154.0.8037.58), the CEF version after the one the app ships (152). The spike commit is `1de415fd1` on a local branch `agent1/omnibox-spike-8037` (not pushed).

---

## 1. Answer

**Yes.** A small patch to the CEF fork exposes Chrome's address bar autocomplete to the embedder. It was built and tested in an Alloy-style browser hosted as a native child window, which is how AgentMux hosts panes on Windows. For every query it returned:

- **history matches**, ranked by Chrome, with typed-visit counts and inline completion (`git` → "git**hub.com**");
- **"search for what you typed"** on the profile's default search engine (Google);
- **live search suggestions** and entity suggestions from that engine;
- **correct URL-or-search classification**: `localhost:3000` is the URL `http://localhost:3000/`. Today's address bar sends it to a Google search.

The first results arrive in about 2 ms and the complete set, including the network suggestions, in about 60–70 ms.

CEF has no API for this because it never wrapped Chrome's product features. Since CEF moved to Chrome's runtime, the code runs in every CEF build, and the profile already records Alloy-style browsing into Chrome's own history. Only the entry point was missing.

## 2. What was tested

### 2.1 Stage 1: no build (CEF 154 `cefclient`, isolated profile)

- In Chrome style, `chrome://omnibox` (Chrome's debug page for the same engine) returned history matches (`HistoryURL`, relevance 1413, `typedCount` 2) and Google search suggestions ("weather in los angeles", "weather in la") for the test profile.
- In Alloy style the same profile recorded typed navigations to a site never visited before (rust-lang.org). Back in Chrome style, the engine returned it from history with `typedCount: 2`. So Alloy-style browsing feeds Chrome's history; the engine is per profile, not per browser style.
- Alloy style refuses `chrome://omnibox` itself (`ERR_ABORTED`). That limits the debug page, not the engine.
- The installed AgentMux instance shows the same: its Personal profile's `Default/History` database is written by its browser panes (row counts only were read).

### 2.2 Stage 2: the patch, built and called

`cefclient --use-alloy-style --use-native --autocomplete-test="git|rust|wiki|weather in|localhost:3000"`, against the stage 1 profile. Final result set for each query (relevance, type, provider):

| Query | Results |
|---|---|
| `git` | 1413 history-url `github.com` (inline "hub.com"); 1390 history-url `github.com/chromiumembedded/cef`; 1301 search-suggest-entity "GitHub"; 1300 search-what-you-typed; search-suggest "github desktop", "gitlab", "gitee", "github copilot" |
| `rust` | 1413 history-url `rust-lang.org` (inline "-lang.org"); 1390 history-url `rust-lang.org/learn`; 1300 search-suggest-entity "Rust"; search-suggest "rustic canyon", "rustdesk", … |
| `wiki` | 1413 history-url `wikipedia.org` (inline "pedia.org"); 1301 search-suggest "wikipedia"; 1300 search-what-you-typed; "wikirace", "wikimedia commons", entity "Wikimedia Foundation", … |
| `weather in` | 851 search-what-you-typed; search-suggest "weather in los angeles", "weather in la", "weather in long beach", "weather in las vegas", … |
| `localhost:3000` | 1203 url-what-you-typed `http://localhost:3000/`; 850 search-what-you-typed |

Every query sent two updates: the local providers at once, then the final set after the network suggestions. No startup errors, no API hash complaints.

## 3. The patch

The commit is `1de415fd1` (fork, local branch `agent1/omnibox-spike-8037`); 4 hand-written files, about 240 lines.

- **Public API** (`include/cef_request_context.h`, `added=next`):
  ```cpp
  /*--cef(source=client,added=next)--*/
  class CefAutocompleteCallback : public virtual CefBaseRefCounted {
    // Called on the UI thread each time the result set changes, best match first.
    virtual void OnAutocompleteResult(CefRefPtr<CefListValue> matches, bool done) = 0;
  };
  // CefRequestContext:
  /*--cef(added=next)--*/
  virtual void GetAutocompleteSuggestions(const CefString& text,
                                          CefRefPtr<CefAutocompleteCallback> callback) = 0;
  ```
  Each match is a dictionary: `contents`, `description`, `destination_url`, `fill_into_edit`, `inline_autocompletion`, `type`, `provider`, `relevance`, `allowed_to_be_default_match`.
- **Implementation** (`libcef/browser/request_context_impl.cc`): on the UI thread, take the context's `Profile` (`CefBrowserContext::AsProfile()`), build an `AutocompleteController` with `ChromeAutocompleteProviderClient(profile)` and the default omnibox providers (minus history embeddings, which allow one query at a time per profile), observe it, report each update, and delete itself after the final one or after 10 s. This is how Chrome's own `chrome://omnibox` page drives the engine (`chrome/browser/ui/webui/omnibox/omnibox_page_handler.cc`).
- **Generated glue** from CEF's translator (C API struct, cpptoc and ctocpp for the callback).
- **Test hook:** `cefclient --autocomplete-test=...`.

No new build dependencies: `libcef` already links Chrome's browser layer through `//chrome:dependencies`.

## 4. What AgentMux would still need

1. **Make the patch production-ready.** Freeze the API version (`version_manager.py`, replacing `added=next`), list the generated files in `cef_paths.gypi` (the spike compiled them through existing files to avoid a regeneration, §6), and carry it to the 152 branch or ship it with 154. It's then one more patch to keep through CEF upgrades; the omnibox internals it calls change from time to time.
2. **Bind it:** add the callback and method to the `agentmuxai/cef-rs` binding fork.
3. **Host:** a route that takes `{block_id, text}`, runs the query on the pane's request context (so a named profile gets its own history and Incognito gets the off-the-record profile), and streams updates back. Cancel the previous query when a new keystroke arrives.
4. **Frontend:** a dropdown under the address bar that shows the matches, accepts the inline completion, and navigates to `destination_url`. It covers the native pane, so it needs the pane overlay clip (`usePaneOverlay`). That costs one clip update per keystroke, not per frame, so it doesn't interfere with the resize work (`ANALYSIS_BROWSER_PANE_RESIZE_ARCHITECTURE_2026_10_06.md`).
5. **Policy and settings:**
   - **Search suggestions** go to the default search engine (Google) with each keystroke, through the same service Chrome uses. Whether Google's terms allow that for a Chromium-based app needs checking before it ships; it should be a setting either way.
   - **Choosing the engine:** the profile's default search engine is Chrome's setting, not exposed in AgentMux today. Exposing it means a little more API, or setting Chrome's preference.
   - **Providers to leave out:** clipboard suggestions, on-device and ML providers, and anything that reaches into Chrome UI the panes don't have (open tabs: Alloy panes aren't in Chrome's tab list, so that provider is simply empty).
6. **Learning from choices.** Chrome's shortcuts provider learns which result the user picked for a given input; that needs the host to report the chosen match back (follow-up).

## 5. Not verified

- **Incognito and named profiles.** The engine takes whichever profile it's given, so named profiles should work as-is. For Incognito, Chrome's own omnibox reads the original profile's history read-only; the patch passes the off-the-record profile as given, and how its providers behave there wasn't tested.
- **macOS and Linux.** Same code, untested.
- **CEF 152.** Built and tested on 154 only; the omnibox APIs used are the same, but the patch hasn't been applied to 152.
- **Long-running load.** Each query creates and discards a controller; Chrome's own omnibox keeps one alive per window. A product version should keep one per pane.

## 6. Building the CEF fork: a finding for the 154 build

The existing CEF 154 build output can't be reused incrementally after any change to a GN input file. The current source tree regenerates the toolchain's include and library paths in relative form (`-imsvc../../../../../../../../Program Files (x86)/…`), while the output was generated with absolute ones (`-imsvcC:\Program Files (x86)\…`). Same compiler and SDK; only the format differs. But every compile command then looks changed, so ninja rebuilds all 52,600 steps. It happens from PowerShell too, so it isn't the shell. The spike avoided it by not adding new files to `cef_paths.gypi`. Any real patch that adds files should expect one full build, or the path format should be fixed first.

The spike built in the shared tree with the owner's approval and restored it afterwards. The original output was set aside unmodified and renamed back, the mirror `src/cef` was restored file by file with original timestamps, and the fork repo is back on `6fba2047e` with no changes. The release artifacts (`release-154-r2`) were never touched.
