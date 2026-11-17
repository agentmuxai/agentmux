# Spec: an agent can have a Vault credential entered wherever one is asked for, and never sees it

**Status:** active. The spec task (#47888) is complete and the build (#47889) is under way: part C is built by the PR that adds "As built (C)" when it is written; parts A and B follow.
**Task:** #47888 (the build is #47889). **Author:** Agent2-6 (Vault). **Date:** 2026-11-16.
**Asked by the operator (2026-11-16):** a container agent signs in to a site with a login the person shared with it, and, more widely, agents can have a Vault credential entered wherever one is asked for, and never see it.
**Public repository.** This spec and its tests use invented names (`example.com`) only.

This spec states requirements and the tests that prove them. It says nothing of how an attacker might try to get round them, and lists no gaps: those go to the security lead privately.

## 1. The rule

An agent can have a Vault credential entered wherever one is asked for: a web sign-in form, a prompt a command shows while it runs, a password field in a desktop app. AgentMux types it. The agent never sees it, never fills it by any other route, and what it can read back afterwards shows the value as hidden.

Everything below is that one rule applied to a place, plus the guarantees every place shares (section 4).

## 2. Scope

In: the three places above (sections 5 to 10), and sharing a login with an agent (section 5), importing a login's user name and password together (section 6), and recording a container agent's Vault calls under its identity (section 7).

Out (section 11): prompts that exist to ask a person (keyring unlock, UAC, polkit, macOS authorisation). One narrow task is named there.

## 3. The places: what exists today, what is missing, what the build adds

| # | Place | Exists today | Missing | The build adds |
|---|-------|--------------|---------|----------------|
| 1a | Web sign-in form, an agent's own browser profile, native or container | `Browser` `signin` types a saved login's user name and password into the sign-in form the page shows now, in a frame of exactly the entry's site, after the person's Allow once / Always for this agent on this site / Deny. It uses only logins whose profile is the tab's profile. | A login the person keeps in Personal cannot be used by an agent's own profile; a container agent cannot use Personal at all. | Sharing a login with a named agent (section 5). |
| 1b | Sign-in flows the form fill does not cover | One form with a password field on one page. | A user name page then a password page; a prompt that asks for the password again; a one-time code; an HTTP sign-in dialog; a form on another site of the same service. | Section 8. |
| 1c | Import into a login | `import` copies one value into one entry. | A login's user name and password from two keys of one secret, in one entry, under one approval. | Section 6. |
| 1d | A container agent's Vault calls | `list`, `show`, `request` and others work; `update` answers 403 "update needs the agent's signed identity". | The call is recorded under the container agent's identity like a native agent's. | Section 7. |
| 2 | Terminal | `Terminal` `run` gives a command a Vault value in its environment (`secrets`), on stdin (`stdin_secret`) or in a file only it reads (`secret_files`), after the person's approval, at the start of the command, locally only. Output is cleaned of the values. | A password prompt that appears while the command runs (`sudo`, `ssh`, a CLI's own login). Delivery for a container agent, in WSL and over SSH. | Section 9. |
| 3 | Desktop apps | `Desktop` `type` refuses a field that UI Automation says is a password field, and tells the agent to ask the person to type it. A browser window and password managers are blocked. | Typing a Vault value into such a field. | Section 10. |
| 4 | OS prompts that ask a person | The person answers them. | Nothing: out of scope. | Section 11. |

## 4. The guarantees every place keeps

Each is a requirement with a test in every place that has a build in this spec (the test name carries the place: `web`, `terminal`, `desktop`).

- **G1 Typed by AgentMux.** The value is read inside srv, from the sealed entry, at the moment of use and no earlier, and written straight into the target. No call returns it, no event, log line, error text or history line holds it, and it is dropped from memory after use.
- **G2 Only where the entry is saved for.** An entry names the site, host or program it is for. A value goes only to a target AgentMux itself identifies as that one (its own record of the page's origin, the process, the host), never to one the agent says is that one. Anything else is refused with a reason that says what to do, and nothing is typed.
- **G3 The person approves, sees it and can stop it.** Every use is asked once (Allow once, or Deny) unless the person granted "Always" for that agent and that place. A use under "Always" shows a notice naming the agent and the place for 5 seconds with a Stop. Only the person can answer or stop: no `UI`, `Browser` or `Desktop` call by an agent reaches the card or the Stop, and neither does input that an agent's program or a `Desktop` call injects at AgentMux's own windows, wherever the platform marks injected input. The card's facts (agent, entry, target, command) are drawn by AgentMux from its own records. The agent's reason is shown as the agent's words: one plain line, no markup and no line breaks, and it cannot occupy the place of the facts or imitate them.
- **G4 Hidden afterwards.** What the agent can read back after a use shows the value as hidden: page snapshots, `find`, `eval` and screenshots of a pane, command output and the pane's log, a desktop screenshot of the window. This covers a password and a one-time code. A login's user name is masked in Vault answers and in `signin`'s messages (B4); the spec promises nothing about a page that shows it. The existing masking rules apply; this spec adds no way round them.
  - **Web.** From the moment a value is typed into a field until that document is gone, no agent script runs in it and no agent call returns the field's value, by any `Browser` action (`eval`, `snapshot`, `find`, a read-back) or any pane screenshot. This includes a field whose type the page switches from password to text. It holds from step 1 on.
- **G5 Recorded.** Each use, each refusal and each stop is a line in the entry's history: when, which agent (its verified identity), the place (origin, host or program), the outcome, the agent's reason. No value, tail, length or hash.
- **G6 Never by another route.** Every other route that could put a login's password or one-time code into a field the agent chooses stays refused (`Browser` `fill` with `vault`, `fill` with `text` for a Vault value, `Terminal` `run` writing a login's password anywhere the agent names other than the means in section 9, `Desktop` `type`), and its refusal names the route that works.
- **G7 Identity is verified.** Every call that asks the person on an agent's behalf, and every use above, verifies which agent asks by the same means for a native and a container agent (section 7). A call whose identity can't be verified is refused and nothing is asked or typed.

Tests of this section (each run once per place that has a build):
`g1_no_response_event_log_or_history_holds_the_value_{web,terminal,desktop}`, `g2_a_target_that_is_not_the_saved_one_gets_nothing_{web,terminal,desktop}`, `g3_a_call_by_an_agent_cannot_answer_or_stop_the_card_{web,terminal,desktop}`, `g3_always_shows_the_notice_and_stop_works_{web,terminal,desktop}`, `g4_readback_shows_hidden_{web,terminal,desktop}`, `g4_web_no_agent_script_runs_and_no_call_returns_the_field_value_even_after_a_type_switch`, `g3_the_reason_cannot_imitate_the_facts_and_injected_input_does_not_answer_the_card`, `g5_a_use_a_refusal_and_a_stop_are_each_one_history_line_{web,terminal,desktop}`, `g6_the_other_routes_still_refuse_and_name_the_one_that_works`, `g7_an_unverified_caller_gets_nothing_asked_or_typed`.

## 5. Part A: sharing a login with an agent (place 1a)

A login entry has a browser profile it belongs to (Personal, or a named profile). Today only tabs of that profile may use it. A **share** lets a named agent's own browser profile use it too, without moving it and without typing the secret again.

- **A1 The share.** A login entry can record the agents it is shared with, by agent id. Sharing changes nothing else about the entry: its profile, site and Vault stay.
- **A2 Who makes it.** The person makes a share: from the entry's card in the Vault ("Share with an agent"), and from the import's approval card (section 6). An agent can ask for one (`Vault` `share` with the entry, itself as the agent, and a reason); the person sees a card naming the agent, the entry's label and site, and the reason, with Allow or Deny. Only their answer makes the share. An agent can neither make nor widen nor remove a share by any other call. An agent can ask for a share **for itself only**: a request naming another agent, or "every agent", is refused. "Every agent" is made only from the entry's card. A share is written by the person's Share and Remove (and by nothing else): not by an agent's `update`, `generate` or `import`, not by importing a file (which never brings one in, and a duplicate it replaces keeps its own), and restoring an earlier value leaves it as it is.
- **A3 Agents generally.** A share can name "every agent" instead of one. Each agent's first use still asks (A6), so "every agent" widens who may be asked, never who is let in unasked.
- **A4 Where a shared login is used.** In a tab of the sharing agent's own browser profile, native or container. It is not used in any other agent's tab, in another profile's tab, or in an Incognito tab. A container agent can use a shared login and no other login of Personal: in an Incognito tab or a Personal one it is offered none, and the refusal says to sign in from its own profile's tab with a login the person shared.
- **A5 Not a way round the Vault's agent list.** A login in a Vault the agent may not use is not offered, shared or not. Making a share with an agent the entry's Vault is closed to is refused with a reason that says to open the Vault to it first.
- **A6 Approval on use.** A share lets the agent ask; it does not answer for the person. The first `signin` by an agent with a shared login on a site asks as `signin` does today: Allow once, Always for this agent on this site, or Deny. "Always" is kept per entry, agent, profile and site, and is asked again when the entry's site or profile is edited. A share made again starts by asking: any "Always" the agent held for the entry ends when the share is made. Why not asked every time: an agent signing in again after a session ends would otherwise stop waiting for a person; the notice, its Stop and the history line are what keep each use visible.
- **A7 Visible and removable.** The entry's card lists each share (agent, since when, last used, number of uses) with a Remove. Removing a share works whatever the login has become since (a login edited to have a realm can still be unshared), and takes effect before the next use, ends any "Always" under it, answers a card waiting on it No, and binds every request to the entry as the person was shown it: a card or notice still waiting (a share removed and made again included) types nothing and is given no "Always". The Vault's list marks an entry that is shared. The Audit view's Secrets section and the entry's history show each use.
- **A8 What the agent is told.** `Vault` `list` and `show` say an entry is shared with the agent (never with whom else: the entry's history records a share as "shared with an agent", and the entry's card lists whom for the person). `Browser` `signin` names the entry used and the site, as now, and `"password": "[secret: hidden]"`. The error when a share is missing says the entry is not shared with this agent and that the agent can ask for it with `Vault` `share`.
- **A9 A share goes with the entry.** Deleting the entry removes its shares. Deleting an agent's browser profile or the agent does not delete the entry; it removes the shares naming that agent and ends every "Always" it holds.
- **A10 The agent is named by its permanent uid.** A share and an "Always" name the agent by its permanent uid, never by a name or a slug. A renamed agent keeps them; a new agent that takes a deleted agent's name or slug gets none.

Tests: `a1_a_share_leaves_profile_site_and_vault_as_they_were`, `a2_only_the_persons_answer_makes_a_share`, `a2_an_agent_cannot_make_widen_or_remove_a_share_by_any_other_call`, `a3_every_agent_still_asks_on_first_use`, `a4_a_container_agent_signs_in_with_a_shared_login_in_its_own_profile`, `a4_not_in_another_agents_tab_another_profile_or_incognito`, `a4_a_container_agent_cannot_use_an_unshared_personal_login`, `a5_a_share_to_an_agent_the_vault_is_closed_to_is_refused`, `a6_first_use_asks_and_always_is_kept_per_entry_agent_profile_and_site`, `a6_editing_the_entrys_site_asks_again`, `a7_removing_a_share_stops_a_card_waiting_on_it`, `a7_a_share_removed_and_made_again_while_asked_types_nothing_and_gives_no_always`, `a8_list_and_show_say_shared_with_you_and_not_with_whom`, `a9_deleting_the_entry_removes_its_shares`, `a9_deleting_the_agent_removes_its_shares_and_ends_its_always`, `a10_a_renamed_agent_keeps_its_share_and_a_new_agent_with_the_old_name_gets_none`, `a2_an_agent_can_ask_for_a_share_for_itself_only`, `a2_generate_cannot_make_a_login_that_is_already_shared`, `a2_an_import_never_carries_a_share_in_and_a_replaced_login_keeps_its_own`, `a2_restoring_an_earlier_value_leaves_who_it_is_shared_with_as_it_is`, `a4_a_container_agents_incognito_tab_is_offered_no_personal_login`, `a7_a_share_made_again_asks_again`, `a7_a_share_can_be_removed_whatever_the_login_has_become`, `a8_the_history_names_no_agent_to_an_agent_that_reads_it`.

## 6. Part B: one import, one login (place 1c)

- **B1** `Vault` `import` and `import_batch` accept, for a new `login` entry, two keys of one secret: `key` (the password) and `user_key` (the user name). One approval names both keys. The person is asked once, as for any import.
- **B2** The user name lands in the entry's `username` field and the password in its sealed password part. The import's answer holds the entry's id and label, never either value.
- **B3** `site` and `share` can be given with the import. A site is checked as any login's site is (an exact `https` origin). A share given with the import appears on the approval card as a plain line ("`<agent>` will be able to sign in to `<site>` with this login") the person can untick; unticked, no share is made.
- **B4** A login's user name is shown to an agent masked (the first two characters and `***`), in `list` and `show`, as `Browser` `signin` already shows it in its messages. An agent can still set a user name with `generate` and `update`; it can't read one back. The Vault pane and the person see it whole.
- **B5** A key that is missing, or holds a value that is not text, refuses the whole item before anything is written, naming the key. `user_key` without `key`, or either on a kind other than `login`, is refused.
- **B6** The entry's history line names the secret, both keys, the Vault and the agent's reason; no value.

Tests: `b1_two_keys_one_approval_one_entry`, `b2_username_field_set_and_password_sealed_and_answer_holds_neither`, `b3_site_is_checked_and_an_unticked_share_makes_none`, `b4_an_agent_reads_the_user_name_masked_and_can_still_write_it`, `b5_a_missing_key_or_a_wrong_kind_refuses_before_anything_is_written`, `b6_the_history_line_names_both_keys_and_no_value`.

## 7. Part C: a container agent's calls are its own (place 1d)

A container agent holds a token and no signing key, and is proven by its token, as `Browser` proves it.

- **C1** One function verifies an agent's identity for every call that records or asks on an agent's behalf (Vault `update`, `request`, `generate`, `rotate`, `import`, `import_batch`, `check`, `history`, `share`; `Terminal` `run` with `secrets`, `stdin_secret`, `secret_files`, `capture` and prompt answers), and it accepts exactly what `Browser`'s accepts: the caller's token for a container agent, the signature for a native one. A container agent's token is accepted only for the agent it was issued to, and only while that agent's container is the caller AgentMux expects (the same peer check its other brokered routes use). The token of a stopped, recreated or deleted agent is refused.
- **C2** A container agent's `update` of an entry's non-secret fields (its label, tags, notes and the other fields C6 does not name) is therefore recorded under its identity, in the entry's history, as a native agent's is. It is not refused.
- **C3** What stays the person's: a login's browser profile and proxy. An agent's `update` that sets them is refused with a reason that says the person sets them in the entry's card (as today for a native agent) and that a share (section 5) is how an agent's own profile uses a login.
- **C4** A call that changes, uses or copies something and whose identity can't be verified is refused, whatever the caller, with a reason naming what to do, and asks and records nothing (`update`, `generate`, `rotate`, `import`, `import_batch`, `check`, and every use in sections 5 to 10). The read routes (`list`, `show`, `history`, `verify`, `health`) answer such a caller as they answer one who sends no identity: only the Vaults open to every agent. A `request` from one is shown to the person as coming from something that is not a verified agent, and makes no entry.
- **C5** The same agent's native and container calls to the same route get the same answer for the same entry.
- **C6 Where an entry may be used is not an agent's to change.** The fields that say where an entry may be used (its site, the further sites of D5, the ssh host, the program, the app) are set by an agent only in the call that creates the entry, where the person's card names them. On an existing entry, an agent's change to any of them is refused with a reason that says the person sets it in the entry's card, and the old target stands (today's rule for the site, realm, profile and proxy, which this spec keeps and extends to each field it adds). A change the person makes themselves, or confirms on a card that shows the old and the new target, ends every "Always" on the entry, and every share waits until they have answered.

Tests: `c1_every_route_that_names_an_agent_verifies_through_the_one_function` (a list of the routes, checked against it), `c2_a_container_agents_update_is_recorded_under_its_identity`, `c3_profile_and_proxy_stay_the_persons_with_a_reason`, `c4_an_unverified_call_is_refused_and_records_nothing`, `c5_native_and_container_get_the_same_answer`, `c1_a_token_of_a_stopped_recreated_or_deleted_agent_is_refused`, `c1_a_token_for_one_agent_does_not_verify_another`, `c6_an_agents_update_of_site_alone_changes_nothing_and_no_later_signin_goes_to_the_new_site`, `c6_the_persons_yes_on_the_old_and_new_target_card_applies_it_and_ends_every_always`.

## 8. Browser flows `signin` does not cover (place 1b)

`Browser` `signin` stays the one route for a login (G6). `fill` with `vault` keeps refusing a login's password and one-time code, and its error says to use `signin`. The reason is that `signin` checks the page's origin from the host's own record, shows the notice, and records the use; a general fill into any field would need none of that.

- **D1 A user name page, then a password page.** `signin` on a page with a user name field and no password field types the user name only, after approval. One approval covers one user name typing and one password typing, and the card on the first step says the password follows. A `signin` on the next page, for the same entry, pane, agent and origin, types the password with no second question, and the history shows one use with two steps. The window is used up by the password typing or by 5 minutes, whichever comes first; it is not a 5-minute pass. After that, a navigation to another origin, or a different entry, it asks again.
- **D2 Asking for the password again.** On a page of the entry's site with a password field and no user name field (a "confirm it is you" prompt), `signin` types the password only, never into a field that is not a password field, and never a user name.
- **D3 A one-time code.** A login's `totp` part is typed by `signin` with `part: "totp"`, into a field the host knows as a one-time-code field, on the entry's site, asked every time (no "Always"). AgentMux computes the current code inside srv and types only that code, never the seed; the code is typed only into a code field on the entry's exact origin; the agent never sees it. This waits for the person's yes (section 13).
- **D4 An HTTP sign-in dialog.** When a pane an agent drives meets an HTTP sign-in challenge (Basic or Digest) and a login saved for that origin and realm is shared with the agent, the person is asked as for `signin`, and AgentMux answers the challenge itself. The agent is told a challenge was answered, never the value. With no share, the agent is told which entry to ask for. Only for `https`; never for a proxy's challenge; only for a top-level navigation to the entry's origin, or a same-origin request of such a page.
- **D5 A form on another site of the same service.** A login can list further sites it is also used on (each an exact `https` origin, added by the person in the entry's card). A page whose origin is one of them is offered the login; no other origin is, and no site is matched by a guess from the name.
- **D6 `fill` stays refused.** See above.

Tests: `d1_two_page_sign_in_is_one_approval_and_one_history_use`, `d1_expiry_origin_change_or_other_entry_asks_again`, `d1_the_window_is_used_up_by_the_password_typing`, `d1_the_first_card_says_the_password_follows`, `d4_http_only_https_never_a_proxy_challenge_and_only_same_origin_requests`, `d3_only_the_current_code_is_typed_never_the_seed`, `d2_password_only_prompt_types_the_password_and_no_user_name`, `d3_totp_is_typed_only_into_a_code_field_asked_every_time`, `d3_the_code_never_leaves_srv`, `d4_an_http_challenge_is_answered_after_the_person_says_yes`, `d4_without_a_share_the_agent_is_told_to_ask`, `d5_only_a_listed_origin_is_offered_the_login`, `d6_fill_with_vault_refuses_a_logins_password_and_names_signin`.

## 9. Terminal prompts (place 2)

What exists stays as it is. This adds an answer to a prompt that appears while a command runs.

- **E0 Which programs.** This build answers prompts for an `ssh` sign-in to a host, and for another named program only where this spec says, for that program, how AgentMux itself knows where the program sends the value. A program name alone is not a destination (G2). Any program without such a rule hands off. `sudo` is not in this build (E12).
- **E1 The call.** `Terminal` `run` takes `answers`: a list of `{entry, part, for}`, where `for` names the program that will ask (`ssh`, or a program with a rule under E0) and, for `ssh`, the host. Not a text pattern the agent writes.
- **E1a One program, directly.** `answers` is accepted only when the command is the named program run directly: its first word is that program and the command line has no shell operator that joins another command (`;`, `&&`, `||`, a pipe, a subshell, a redirect into a command). Anything else is refused with a reason that says to run the program alone.
- **E2 What is typed to.** AgentMux types the value only when it sees, in the process it started, a prompt: the named program is running in it (its executable's resolved path read from the process table; the executable and every folder above it must not be writable by the user the agent runs as, checked at use, so a program the command's folder or a `PATH` entry in the agent's workspace supplies gets nothing; where that is not true, including a WSL distribution where the agent is root, it hands off) and its terminal is asking for input with echo off. Where AgentMux cannot tell that (a terminal that does not report echo), it types nothing, tells the agent, and the person types it (a hand-off).
- **E3 Saved for.** The entry is saved for the program and, for `ssh`, the host (its `site` reads `ssh://host` or `program:name`). A prompt of another program or host gets nothing.
- **E4 Approval.** The card shows the entry, the agent, the whole command, where it runs and which program's prompt will be answered. Allow once or Deny, each run; no "Always" in this build. A prompt that appears twice in one run is answered once; a second asks the person.
- **E5 Output.** The typed value is not echoed by the terminal, and the command's output and the pane's log are cleaned of it as for `secrets`.
- **E6 Where.** Local and a local shell first. WSL second, over the same means where the distribution's terminal reports echo. Over SSH (`connection`): only for a prompt of the remote host's own program, with the entry saved for that host, and the card names the host. A container agent: the value is not delivered into a container process (credentials are never written into the container); the agent is told so, and the person types it (a hand-off).
- **E7 Record.** One history line per answered or refused prompt: the program, the host, the outcome.
- **E8 Identity.** As section 7.
- **E9 ssh's destination.** For `ssh` the value is typed only when AgentMux itself has established that the connection is to the entry's host, verified against the person's own known hosts. Options, config files or environment the command supplies that change where it connects, through what (a proxy or jump host), or how the host is checked are refused, with a reason.
- **E10 The program's environment is AgentMux's.** AgentMux builds the environment the program starts with. Nothing the agent supplies that changes the program's loader, its askpass or its prompt path is passed on.
- **E11 Remote prompts.** A prompt from a program on the far side of a `connection` hands off in this build.
- **E12 No `sudo` here.** `sudo` is the person's own account password and root on the host, the same kind of prompt as section 11's. It is not in this build unless the person says so. If they do: per-run approval with the whole command, and an answered `sudo` leaves no cached authorisation that any later command of the agent can use, with a test.
- **E13 Container agents.** A container agent's prompts hand off (E6).

Tests: `e1_a_pattern_the_agent_writes_is_not_accepted`, `e1a_a_compound_command_is_refused`, `e2_a_lookalike_program_in_the_workspace_gets_nothing`, `e2_typed_only_when_the_named_program_asks_with_echo_off`, `e2_unknown_echo_state_hands_off_and_types_nothing`, `e3_another_program_or_host_gets_nothing`, `e4_card_shows_command_program_and_host_and_has_no_always`, `e4_a_second_prompt_in_one_run_asks_again`, `e5_output_and_log_are_cleaned`, `e6_a_container_agent_is_told_to_hand_off_and_nothing_is_written_in_the_container`, `e6_ssh_requires_the_hosts_own_prompt_and_the_entrys_host`, `e7_one_history_line_per_prompt`, `e0_a_program_without_a_destination_rule_hands_off`, `e2_a_program_in_a_folder_the_agents_user_can_write_hands_off`, `e9_ssh_is_answered_only_for_the_entrys_host_found_in_the_persons_known_hosts`, `e9_options_config_or_environment_that_change_the_destination_are_refused`, `e10_nothing_that_changes_the_loader_askpass_or_prompt_path_is_passed_on`, `e11_a_remote_prompt_hands_off`, `e12_sudo_is_not_answered`.

## 10. Desktop apps (place 3)

- **F1 The call.** `Desktop` `type` takes `vault` (an entry and part) in place of `text`.
- **F2 When it types.** Only when the host's UI Automation (or the platform's equivalent) says the focused control is a password field; an unknown or ordinary field is refused with a reason that says the person types it. Today's guard on `text` stays as it is.
- **F3 Saved for.** The entry names the app by its full path and, where the platform has one, its code signer (`site` reads `app:<path>`, with the signer). Both are on the card and both are checked at use. AgentMux reads the focused window's program from the operating system, not from the window's title, and types only into the app the entry names. An unsigned program in a folder the agent's user can write to hands off.
- **F4 Never in a browser or a password manager.** The existing blocklist stays: a browser window is typed into through `Browser` `signin` only.
- **F5 Approval, notice, stop, record.** As G3 and G5, with the app's name on the card. No "Always" in this build. A global stop key and the on-screen indicator that `Desktop` already has stop it.
- **F6 Hidden afterwards.** After a Vault value is typed into a window, every `Desktop` read of that control (a screenshot region, the tree's text, its value) shows it hidden for as long as the control holds it, including after a "show password" toggle changes what kind of field the control reports. A screenshot that would show the value is refused with a reason.
- **F7 Platforms.** Windows first. macOS and Linux follow where the platform reports a secure field; where it does not, a hand-off.

Tests: `f1_vault_in_place_of_text`, `f2_only_a_reported_password_field_gets_a_value`, `f3_only_the_apps_own_program_gets_it`, `f3_path_and_signer_are_on_the_card_and_checked_at_use`, `f3_an_unsigned_program_in_a_writable_folder_hands_off`, `f6_every_read_of_the_control_stays_hidden_after_a_show_password_toggle`, `f4_a_browser_window_is_refused_and_names_signin`, `f5_card_notice_stop_and_history`, `f6_a_screenshot_with_a_shown_value_is_refused`.

## 11. Out of scope: OS prompts that exist to ask a person

| Prompt | Why it stays the person's |
|--------|---------------------------|
| Keyring or Keychain unlock | It guards the key to every stored secret; answering it for the person would make the Vault's own key reachable by what it protects against. |
| UAC (Windows) | Its secure desktop exists to be unreachable by programs. |
| polkit (Linux) | It asks the logged-in person to approve a change to the system. |
| macOS authorisation (Touch ID, password sheet) | It is the system's own proof that a person is present. |

One narrow task worth its own ticket: a Vault key store that stays locked after an unattended restart (a Linux Secret Service whose keyring has no session unlock, a headless macOS login). The app should say so on start, say what to do, and keep working for everything that needs no secret. No bypass.

## 12. Build order (#47889)

1. **Pulp's case:** sections 7 (C, including C6), 5 (A, including A10) and 6 (B), and the web read-back guarantee of G4, with the Vault pane's share list, the import card's line and `Vault` `share`. A container agent signs in on a site with a shared login.
2. **Browser gaps:** D1, D2, D5, then D4, then D3.
3. **Terminal prompts:** E, local first.
4. **Desktop:** F, Windows first.
5. The locked-keyring task, filed apart.

Each step is one PR with its tests and a Vault pane look check on macOS and Linux for every visible change (the share list, the cards).

## 13. Choices this spec makes, for the person to confirm

- Approval on use of a shared login: Allow once or Always for the agent on the site, as `signin` asks today (A6). Terminal prompts and desktop typing: each time, no "Always" (E4, F5). A one-time code: each time (D3).
- A login's user name is masked to agents (B4).
- A container agent's terminal prompts are handed off, not answered, so no value is written into the container (E6).
- One-time codes (D3) wait for the person's yes; so does whether `sudo` is answered at all (E12). Terminal prompts (section 9) start only with the additions E0 to E13.
