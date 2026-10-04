# SPEC — Armory Accounts: Delete in the right-click menu, account details in an inline panel

**Status:** implemented (#4317) — phases 1 and 2; see §6 for where the build differs from the design.
**Trigger:** owner request, 2026-10-04: "add a delete entry to the right click menu on the Armory → Accounts's Connected Accounts. We want to be able to delete the auth entries. Think of edge cases and graceful solutions." Then: "we don't want account details in a modal. Instead, it should slip out as a panel from underneath the account."
**Scope:** the Connected accounts list in Armory → Accounts (`AccountsManager` → `AccountsTab`, `AccountRow`, `AccountDetail`), the account row's right-click menu, and the `deleteidentityaccount` handler and the secret cleanup it runs. More Armory Accounts changes are expected; add them as further sections here.
**Relation to other specs:**
- Replaces §2.1 point 2 ("View existing account" in a list/detail page) of `SPEC_ARMORY_ACCOUNTS_NO_MODALS_2026_07_16.md`. The owner chose an inline panel under the row, not a separate detail page. That spec's other points (connect flow, add form) are untouched.
- Extends `SPEC_ARMORY_BIND_TO_AGENT_CONTEXT_MENU_2026_08_09.md` (the current menu) and `SPEC_ACCOUNT_DELETE_DEAUTH_LAYERS_2_4_2026_07_14.md` (spawn gate, affected-agent events, delete disclosure).
- Implements R5 of `SPEC_SHARED_AUTH_ACROSS_CHANNELS_2026_10_03.md` (confirmation for destructive auth operations from a non-`stable` channel) for account delete.
**Verified against:** `main` @ `decf4c21b`, 2026-10-04.

---

## 1. Today

| Piece | Where | Behaviour |
|---|---|---|
| Row right-click menu | `identity/bind-to-agent-menu.ts` `buildAccountRowMenu` | **Bind to Agent ▸**, separator, **Copy account ID**. No delete. |
| Account details | `identity/identity-accounts-tab.tsx` `AccountsTab` | Clicking a row sets `selectedAccountAtom`; a window-scoped `<Modal size="md">` shows `AccountDetail`. |
| Delete | `AccountDetail` footer | A red **Delete** button behind `window.confirm()`, with a count of linked agents. |
| Delete RPC | `server/agent_handlers/identity.rs` `deleteidentityaccount` | Cleans up the secret, deletes the row, removes the links and the mirror row, publishes `identityaccounts:changed` plus per-agent `agentidentities:changed:<id>` and `agentcredentials:revoked:<id>`, and returns `{deleted, affectedAgents}`. |
| Secret cleanup | `identity/cleanup.rs` `cleanup_account_secrets` | Keychain: deletes the entry. OAuth config dir: `remove_dir_all` on the **whole folder**, only if it sits strictly inside `identities_dir()`. Best-effort: a failure is logged, never returned. |
| Errors | `identity-model.ts` `deleteAccount` | A failure is written to `formError`, which shows only inside the add/edit form. When the form is closed, a failed delete is silent. |

## 2. Hazards found in the current delete

These apply to the existing button as much as to the new menu item, and adding a one-click path to delete makes them more likely, so they are fixed here.

### H1. Deleting a Claude (or Codex, Gemini) account deletes its conversation history

For a CLI-OAuth account, `CLAUDE_CONFIG_DIR` / `CODEX_HOME` is `identities_dir()/<account_id>/<provider>/`, and the CLI keeps its transcripts there (`projects/` for Claude, `sessions/` for Codex, `history/` for Gemini; `ProviderConfig::history_native_subdir`). With the default shared auth, `identities_dir()` is `~/.agentmux/shared/identities`, so that folder holds the real transcripts. Checked on a real machine: the account folders contain `projects/` next to the credential files (`settings.json`, `sessions/`, `backups/`, …).

`cleanup_oauth_dir` removes the whole folder, so **every conversation that ran under the account is deleted with it**, and nothing tells the user.

`SPEC_AGENT_IDENTITY_HISTORY_PERSISTENCE_PROTOCOL_2026_08_16.md` P1 classifies history and credentials as separate persistence categories. Delete must remove the credential and keep the history.

### H2. A failed delete is invisible

`deleteAccount` puts the error in `formError`. From the detail modal (and from a menu item) the form is not open, so the user sees nothing and the row stays.

### H3. Cleanup failures are swallowed

The row is deleted even when the secret could not be removed (keychain access denied or locked, a file in use on Windows because a running CLI holds it open, a folder outside `identities_dir()`). The user is told the account is gone while its token is still on disk or in the keychain.

### H4. Stale references survive the delete

The delete removes link rows but leaves:
- the legacy `db_agents.accounts` JSON (`{"github": "<account id>"}`), still read by `agentsAssignedToAccount`;
- an instance's `identity_id`, which now stores an account id (`storage/agents.rs`);
- a block's `cmd:env` override set by Bind to Agent (`CLAUDE_CONFIG_DIR=<the deleted folder>`). On the next restart that pane points the CLI at a folder that no longer exists. The CLI then creates an empty one and asks for a fresh login, instead of the spawn gate saying the account was deleted.

### H5. Shared login, local-looking action

With shared auth, every AgentMux build and channel on the machine sees the same account. Deleting it from a dev or `local-…` build deletes it for the stable app too (R5 of the shared-auth spec).

## 3. Design

### 3.1 Right-click menu

```
Bind to Agent            ▸
─────────────────────────
Copy account ID
─────────────────────────
Delete account…
```

- **Label:** "Delete account…", last, after a separator, so it is never the item under the pointer when the menu opens. The ellipsis says a confirmation follows.
- **Always enabled** for rows in the list. The AgentMux Cloud row is not an `IdentityAccount` and has no right-click menu today. It gets none from this spec (see §6, Q1).
- **Re-resolve by id on click.** The menu is built from a snapshot. When the item is chosen, look the account up again in `model.accountsAtom()`. If it is gone (deleted from another window or channel since the menu opened), do nothing and show "This account was already removed."
- The menu opens the same confirmation as the panel's Delete button (§3.3). There is one delete flow, reached from two places.

### 3.2 Inline detail panel

Clicking a row opens its details in a panel that slides open **directly under that row**, pushing the rows below it down. It replaces the `<Modal>`.

- **One open at a time.** Opening another row's panel closes the first. `selectedAccountAtom` already models "which one", so it becomes "which row is expanded".
- **Toggle:** clicking the open row again, or pressing Esc while focus is inside the panel, closes it. Clicking elsewhere on the page does not, so the user can copy a value out of it.
- **Motion:** height and opacity, about 150 ms, ease-out. Under `prefers-reduced-motion`, no animation, only open and closed.
- **Look:** the expanded row and its panel read as one card. The row keeps its selected background, the panel shares its left edge and indent, and a chevron on the row turns from ▸ to ▾. The panel has no title bar: the row above is its title.
- **Content:** what `AccountDetail` shows today (status, Provider, Kind, Secret, GitHub / AWS / Anthropic context, Notes, Created) minus `ModalHeader`, since the row already shows the label. Add **Used by:** the linked agents' names (the list the delete disclosure already computes), each clickable to reveal that agent. That replaces today's dead "Assigned via Identities tab" line.
- **Actions:** a row of buttons at the bottom of the panel: Reauth / Validate… (as today, conditional), Edit, and Delete account… (danger style, right-aligned, apart from the others).
- **Scroll:** on open, scroll just enough to bring the whole panel into view (`scrollIntoView({block: "nearest"})`), never jump the row to the top.
- **Keyboard and accessibility:** the row is a button with `aria-expanded` and `aria-controls` pointing at the panel. Enter or Space toggles. On open, focus stays on the row; Tab moves into the panel.
- **Live updates:** the panel reads the account from the cache, so a rename or status change from elsewhere updates it in place. If the account disappears (deleted elsewhere), the panel collapses. If that happened while the user was looking at it, show "This account was removed." in the list's notice area.
- **Edit:** stays as today in this phase (the add/edit form overlay). Phase 2 moves the edit form into the same panel (§5).

### 3.3 Confirmation

Replace `window.confirm()` with `ConfirmModal` (`destructive`, which focuses Cancel), scoped to the Armory tab (`scope="tab"`, as `AccountsManager` already does). One component, used by both the menu item and the panel button.

- **Title:** `Delete <label>?`, where label is `accountLabel(account)` (the login email where there is one).
- **Body**, only the lines that apply:
  - Always: "AgentMux forgets this account and removes its saved login from this computer."
  - Linked agents: "Used by 3 agents: AgentA, Lark, Opaz. They won't start until you bind another account. Any that are running keep working until they restart."
  - CLI-OAuth accounts with a history folder (Claude, Codex, Gemini; H1): "Its conversation history is not deleted."
  - Key and token accounts (`keychain`, `env`, `secrets_manager`): "The key itself still works. To revoke it, do that at <provider> (link)." AgentMux never revokes provider-side (deauth spec §5), and saying so stops the user assuming it did.
  - A folder outside `identities_dir()` (for example a legacy `~/.claude` login, or a row adopted from another channel whose folder is under `channels/<old>/identities/`): "Its login files are outside AgentMux's folder and are left in place: <path>."
  - Non-`stable` channel (H5): "This login is shared by every AgentMux on this computer, including the main app."
- **Buttons:** Cancel (focused), **Delete account** (red). While the RPC runs, the confirm button shows a pending state and the dialog cannot be dismissed. It closes when the RPC returns.
- **No undo.** A real undo would mean deferring the delete for several seconds, and deleting a credential is the one thing that should not linger. A deferred delete can also be lost if the app quits during the window. The confirmation is the safeguard.

### 3.4 Backend changes

1. **Keep history (H1).** `cleanup_oauth_dir` removes everything in the folder **except** the provider's history subdirectory (`history_native_subdir`), plus anything that subdirectory links to. If only the history is left afterwards, keep the folder. The orphan sweep (`sweep_orphaned_account_dirs`) must apply the same rule: today it removes an orphaned folder outright.
   - Where history is already redirected to the always-global `identity_history_dir` (isolated auth), the redirect target is outside the removed tree and is unaffected. Remove the link, not its target. `remove_dir_all` follows no links on Windows junctions or Unix symlinks, but this needs a test with a junction on Windows.
   - Making the kept transcripts visible after the account is gone (for example under the agent's history in the Armory) is out of scope here. Today they are simply kept on disk where the history store already reads them.
2. **Report the cleanup (H3).** The response gains a `cleanup` object:
   ```json
   { "deleted": true, "affectedAgents": ["…"],
     "cleanup": { "outcome": "removed" | "absent" | "skipped" | "failed",
                  "detail": "…", "path": "…", "historyKept": true } }
   ```
   The row is still deleted when cleanup fails. A row the user asked to delete must not come back. The UI reports a cleanup failure (§3.5), and the orphan sweep retries removing the folder later.
3. **Clear stale references (H4)**, in the same handler, best-effort and logged under `identity.delete:`:
   - remove the account from every `db_agents.accounts` JSON blob that names it;
   - clear an instance's `identity_id` that equals the deleted id;
   - for every block whose `cmd:env` has a `CLAUDE_CONFIG_DIR` / `CODEX_HOME` equal to the deleted folder, remove that key, so the next spawn goes through the resolver and its spawn gate.
4. **Idempotent.** Deleting an id that no longer exists returns `{deleted: false}` with no error (it does today). The UI treats it as "already removed", not as a failure.
5. **Concurrency.** Two deletes of one account racing (two windows, or two channels on a shared store): the second finds no row and takes the path in 4. Cleanup tolerates a folder that vanished between `exists()` and `remove_dir_all` (treat `NotFound` as `absent`).
6. **A login in progress.** If a `ClaudeLoginPanel` re-login for this account is running (`AuthStart` with `existingAccountId`), cancel it before cleanup. Otherwise it writes fresh tokens into the folder being deleted, or recreates it afterwards.

### 3.5 Feedback after delete

Replace the `formError` path (H2) with the list's notice area (`deleteNoticeAtom`, rendered above the list), which is visible whether or not anything is open:

| Result | Notice |
|---|---|
| Deleted, cleanup removed or absent, no agents | none; the row disappearing is the feedback |
| Deleted, with affected agents | today's disclosure: "Removed. 3 agents used it and won't start until you bind another account; any running keep working until they restart." |
| Deleted, cleanup failed | warning: "Removed, but its login files couldn't all be deleted (<detail>). AgentMux will retry; you can also delete <path>." |
| Deleted, cleanup skipped (outside AgentMux's folder) | info: "Removed. Its login files are outside AgentMux's folder and were left in place: <path>." |
| `deleted: false` | info: "This account was already removed." |
| RPC error | error: "Couldn't delete <label>: <message>." The row stays. |

### 3.6 Edge cases

| Case | Behaviour |
|---|---|
| The account is bound to a **running** agent | Allowed. The confirmation names the agent; its pane shows the existing "credentials revoked" chip; it keeps running until restart; the next spawn is refused by the spawn gate with "account deleted, bind another". |
| The account is the one **this Armory's own agent** runs on | Same as above; nothing special. |
| The **last** account for a provider, with bound agents | Same, but the notice also offers **Add account** for that provider. |
| The panel for this account is open | It collapses as the row leaves the list. |
| The edit form for this account is open with unsaved changes | Deleting from the menu discards the edit form; the confirmation adds "Unsaved changes to this account will be lost." |
| The menu is open when the account is deleted elsewhere | The item re-resolves by id and reports "already removed" (§3.1). |
| Windows: a CLI process holds files in the folder open | Cleanup removes what it can and returns `failed` with the locked path; the sweep retries after the process exits. |
| macOS: keychain prompt denied, or keychain locked | Cleanup returns `failed` ("keychain access denied"); the row is deleted; the notice says the key may still be in the keychain under AgentMux. |
| An account adopted from another channel, folder under `channels/<old>/identities/` | Outside `identities_dir()`: cleanup is `skipped`, the folder stays, and the confirmation says so up front. Removing other channels' folders is the pruner's job (shared-auth spec §5.2). |
| Isolated-auth dev channel | `identities_dir()` is the channel's own; delete affects only that channel; the shared-login warning is not shown. |
| A store that failed to open and fell back | Deleting from a fallback store cannot reach the shared row. Out of scope here; the shared-auth spec's "cannot open the shared store" notice covers it. |
| Several rows selected | Not supported; the list has no multi-select. |

## 4. Tests

- Menu: "Delete account…" is last, after a separator; choosing it opens the confirmation; a stale menu on a deleted account reports "already removed" and calls no RPC.
- Confirmation: lines appear only when they apply (agents, history kept, provider revoke, outside folder, shared login); Cancel is focused; the pending state blocks dismissal.
- Panel: opens under the clicked row, one at a time, toggles on the row and Esc, collapses when its account disappears, respects reduced motion, `aria-expanded` is correct.
- Rust, `cleanup.rs`: history subdir kept and credential files removed for each provider in `history_native_subdir`; a junction or symlink to the global history is removed without touching its target; `NotFound` mid-removal is `absent`; the sweep keeps history.
- Rust, handler: `cleanup` is returned for each outcome; legacy `accounts` JSON, `identity_id` and matching `cmd:env` keys are cleared; a second delete returns `deleted: false`.
- Model: an RPC error and a cleanup failure both reach `deleteNoticeAtom` with the form closed.

## 5. Phases

| Phase | What |
|---|---|
| 1 | Backend: keep history (§3.4.1, the sweep too), `cleanup` in the response, stale references, login-in-progress cancel. Ship first, because it fixes the existing button's data loss. |
| 2 | Frontend: menu item, `ConfirmModal` flow with the conditional lines, notices (§3.5), inline panel replacing the modal. |
| 3 | Edit inside the panel instead of the form overlay, so no modal opens from Accounts' rows at all. |

## 6. As built

Phases 1 and 2 are built. Where the build differs from §3, this section wins:

- **Stale references (§3.4.3, H4): no change needed.** The layer-3 spawn gate (`resolver/inject.rs`) refuses an OAuth-class agent whose binding was cascaded away, before a stale `cmd:env` config dir could be used. So the leftover `cmd:env`, the legacy `db_agents.accounts` entry and an instance's `identity_id` are inert. They name an account id that no longer exists and is never reissued.
- **A re-login in progress (§3.4.6): not cancelled.** If a re-login for the account finishes after the delete, it re-creates the account with a fresh login (`identity_auth_persist.rs` upserts it). That is visible in the list and harmless, so no cancellation plumbing was added.
- **Confirmation lines (§3.3):**
  - The "shared by every AgentMux on this computer" line is not shown. The frontend does not know its channel or whether auth is isolated, and exposing that is a follow-up.
  - The "login files outside AgentMux's folder" case is reported after the delete, from the backend's `skipped` outcome, not predicted in the confirmation.
- **Panel (§3.2):**
  - "Used by" lists agent names; they are not links yet.
  - Closing the panel is instant. Opening it animates through `@starting-style` and `interpolate-size`.
- **Feedback (§3.5):** the last-account "Add account" offer is not built.
- **Edit (phase 3)** still opens the form overlay.

The history rule is in `identity/cleanup.rs`: `remove_credentials_keep_history` for a delete, and `prune_orphan_keeping_history` for the orphan sweep, which would otherwise delete the kept history a few minutes later.

## 7. Open questions

1. **The AgentMux Cloud row.** It has no right-click menu. Should it get one with "Sign out…" (MuxBus sign-out, which clears the shared session for every channel) for symmetry? Recommended: yes, as a separate item with its own confirmation, not "Delete".
2. **Showing kept history.** After a delete, should the Armory say where the kept transcripts are, or offer "Delete history too" as an explicit second checkbox in the confirmation (unchecked by default)? Recommended: offer the checkbox in a later phase, not now.
