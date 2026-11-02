# SPEC: Google Phase B: AgentMux's own client, so a person clicks Connect, signs in and consents

**Status:** active, 2026-11-01. Spec task #16989 (done), build #16990 (Naki-2). The operator approved it in conversation ("yes proceed with phase b").
**Author:** Naki-2@narko.
**Trigger (operator, 2026-11-01):** "ok, we dont need to work on the "BYO" anymore, it isnt a real option for users. lets focus on Phase B". Goal: a user clicks **Connect Google**, signs in and consents. No Cloud project, no client file, no "unverified app" warning.
**Verified on:** `main` at `e4f374c61` (2026-11-01), AgentMux 0.60.10. Google's rules were read on Google's own pages on 2026-11-01 (§2); a statement from anywhere else is marked.
**Extends:** `SPEC_GOOGLE_CONNECTORS_GMAIL_CALENDAR_DRIVE_2026_10_14.md` (the Google spec), whose D8 ("three ways to supply the client") this replaces, and whose "Phase B" it is. **Supersedes** `SPEC_AGENTS_CONNECT_GOOGLE_END_TO_END_2026_11_01.md` (the BYO end-to-end draft, #16317, cancelled): §7 says what of it carries over.

## 1. Summary

1. **AgentMux ships its own Google Desktop client.** Its id is a constant in the app. Its secret, which Google says isn't confidential for a desktop app but which the public repository must not hold, is put in at build time. Every install of a release build connects with it; nobody creates a Google project.
2. **The client lives in a new project, `agentmux-connectors`** (D1), not in `agentmux-prod`, so a review of Calendar or Gmail can never touch the live sign-in.
3. **Google's three tiers decide the order, and the dates are Google's, not ours** (§2): Drive's file picker (`drive.file`, non-sensitive) works for every user as soon as the project's brand is verified, about **2-3 business days** after we ask. Calendar (sensitive) needs a review Google puts at about **10 business days**. Gmail (restricted) needs a review of about **6 weeks** and a **yearly paid security assessment**: Google's pages condition it on server storage or transmission, but Google's help page has no such condition and a local desktop mail app was told this year that local apps are not exempt, so **plan on it** (§6, the operator's decision; Tier 2 quotes run about $540 to $1,800 a year). Until a tier is verified, builds don't ask for it (§8), so no user meets the unverified-app screen or the 100-user cap.
4. **Agents do the console work and write the submission; a person signs in, uploads the video and presses Submit** (§5.5). Nothing is submitted and nothing is paid without the operator.
5. **What the BYO spec built for agents carries over:** the `Google` tool, grants by agents (decided: "yes, agents can grant"), status with reasons, the tool appearing without a relaunch, and the edge cases (§7). Client files, the Vault client kind and console automation for users are dropped.
6. **Gmail is the one open question** for the operator: plan on the assessment, get a quote, and spend nothing before the operator says (§6).

## 2. What Google requires (read on Google's pages, 2026-11-01)

### 2.1 The three tiers

| Tier | Scopes AgentMux asks for | Google requires | Google's own time estimate |
|---|---|---|---|
| Non-sensitive | `openid email profile` (live); **`drive.file`** (the file picker) | Brand verification: homepage, privacy policy and domain proof | 2-3 business days |
| Sensitive | **Calendar**: `calendar.calendarlist.readonly`, `calendar.events.readonly`, `calendar.events.freebusy` (writes: `calendar.events`, later); `gmail.send` if ever offered alone | The above, plus a justification per scope, why no narrower scope is enough, and a demo video | about 10 business days |
| Restricted | **Gmail**: `gmail.readonly`, `gmail.compose` (and `gmail.modify` only if labelling and trashing stay); Drive `drive.readonly` and `drive` (not requested) | The above, plus the Limited Use policy and an annual security assessment (§6: Google conditions it on a server in the data path, but a local desktop app was told it applies anyway) | about 6 weeks, plus the assessment |

Sources: Gmail and Drive scope tables at developers.google.com/workspace/gmail/api/auth/scopes and /drive/api/guides/api-specific-auth; the verification pages under developers.google.com/identity/protocols/oauth2/production-readiness/ (sensitive and restricted); the verification FAQ at support.google.com/cloud/answer/13463817 (the time estimates, "not guaranteed"). Two limits of what was read:
- **Calendar's four scopes carry no label on any Google page.** Google says only that "reading events stored in Google Calendar" is an example of a sensitive scope. The Data Access page in the Cloud console labels each scope; the build reads it there before this table is final (the first `agentmux-connectors` task, §10).
- **Gmail has no non-restricted way to read mail.** `gmail.metadata` is restricted too; only `gmail.send` and `gmail.labels` are not, and neither reads threads. The smallest read set is `gmail.readonly`.

### 2.2 What a verification needs

- **Homepage** on a domain we have verified in Search Console, describing what the app does ("can not be only a login page"), linking the privacy policy. `agentmux.ai` is verified and its homepage and `/privacy` are the live consent-screen links. The new project reuses them.
- **Privacy policy** on the same domain, saying how the app "accesses, uses, stores, and/or shares Google user data", and carrying Google's Limited Use sentence (§5.3).
- **A justification for each scope**, and why a narrower scope is not enough. "Don't attempt to 'future proof' your access": request only what a shipped feature uses (§8).
- **A demo video**, on YouTube as Unlisted, in English: the whole consent flow, the consent screen showing the app name and **the OAuth client id in the browser's address bar**, and each requested scope's feature working. The consent screen in the video must show exactly the scopes submitted.
- **Submitted** in the console under Google Auth Platform, Verification Center, after Branding is published; scopes declared under Data Access.
- **Unverified meanwhile:** an app that asks for a sensitive or restricted scope before approval shows the "unverified app" screen, and Google admits "100 new users in total", for the project's whole life, which can't be reset. Past it, sign-in for those users is disabled. This is why §8 keeps unapproved scopes out of public builds.

### 2.3 What a desktop client is

- **Loopback redirect** (`http://127.0.0.1:<port>`), PKCE, the system browser (an embedded browser is refused with `disallowed_useragent`). All built (G0).
- **The secret:** Google's native-app page calls Desktop apps "public clients" that "cannot keep secrets confidential", and marks `client_secret` "Optional" in the token exchange. The code's comment (`oauth_client.rs`) says Google's token endpoint refuses a code exchange and refresh without it. **The two disagree, and which is true for a new client is tested** (§4). The Desktop client page in the console generates a secret at creation either way.
- **"Use this client for an AI-powered agent"** is a checkbox on the client: a self-declaration, with no extra privilege and still subject to verification. Google asks for separate clients per workflow. Some Google MCP endpoints accept only a client marked this way. AgentMux calls the REST APIs, not those endpoints (Google spec D3), but it is an agent host and G4 may forward to them later, so the client is created with the box ticked. Google's pages disagree on whether the box can be changed after creation, so it is set at creation (§4).
- **Incremental authorization:** the same page says "Incremental authorization is not supported for installed apps or devices." The connectors code asks for each service with `include_granted_scopes=true`. **Untested against a real account** (the first live connect never got that far). §4 gives the safe design: ask for the union of everything the person has on, every time, and treat `include_granted_scopes` as a bonus.

### 2.4 Limited Use, and mail going to a model

The privacy policy and the app need Google's sentence: "{App's} use of information received from Google APIs will adhere to Google API Services User Data Policy, including the Limited Use requirements." The rules that matter here:
- Google user data may not be used "to create, train, or improve a machine learning or artificial intelligence model beyond that specific user's personalized model for the … user-facing feature", nor "stored in conjunction with foundational models". AgentMux trains nothing and keeps no mail: srv makes the call and the text goes to the model the person's agent already runs on.
- **Required security measure** for restricted scopes: "Protecting against prompt injection techniques by either using Google Cloud Platform's Model Armor or other prompt injection protection." AgentMux's measure is the Google spec's §9: content marked as untrusted page content (D6), no raw send, the host window for everything that can't be undone.
- **Mail going to a model provider is the part Google may read as a third party.** Google's pages don't say whether passing a user's mail to the LLM provider the user chose, to answer the user's own request, is the "user-directed appropriate use case" or a transfer to a third-party server. Read together, the rules point to "allowed, with the user's consent, nothing kept or trained on" (a transfer "to provide … user-facing features … and only with the user's consent"; the prohibition is on training and on storing the data "in conjunction with" foundational models), and the "personalized model" definition names on-device models, so an outside provider is not clearly covered. **The submission therefore says it plainly instead of leaving a reviewer to find it** (§5.4).

## 3. What exists (verified in code)

- **`identity/oauth_client.rs`:** `config_for("google")` has `client_id: None` and `requires_secret: true`. `resolve_client` returns a built-in client with `client_secret: None`, and its comment says the built-in "gets one when AgentMux's client is provisioned". `restricted_scope_needs_own_client` refuses Gmail on the built-in client. A saved or typed client (BYO, workspace-internal) is the only way to run today.
- **A developer-build override already exists for GitHub** (`github_connect::client_id_override`, an environment variable): the same pattern serves Google (§4).
- **Refresh** reads a client's secret from the account's token blob (`account_client`). A built-in account's blob has none, so refresh would need the built-in one (§4).
- **The corp record** (a private document): the `agentmux` folder with `agentmux-prod`, `agentmux-staging` and `agentmux-dev`; `agentmux.ai` verified; support group `support@agentmux.ai`; `agentmux-prod`'s consent screen published and brand verified (2026-10-20) for sign-in only; billing not linked. All reused.
- **`agentmux-dev`** (made 2026-11-01): a consent screen with the eight scopes and a Desktop client "AgentMux desktop (dev)". It was in Testing mode (100 listed test users, grants expiring after 7 days); on 2026-11-01 it was **published In production, never submitted for verification** (branding: homepage, privacy, terms, `agentmux.ai`, no logo, so nothing needs brand verification), so internal grants no longer expire. Unverified, it is capped at 100 users and shows Google's unverified-app screen once per account. The Gmail API is on. It is our own sandbox for dogfooding every tier before Google approves it (§8); it is never submitted.

## 4. Decisions

**D1. A separate project for the connectors.** Create **`agentmux-connectors`** in the `agentmux` folder: its own consent screen (name "AgentMux", the same homepage, privacy, terms and support address), brand verification (2-3 business days), and the Desktop client with "Use this client for an AI-powered agent" ticked. Why not `agentmux-prod`: verification state, the 100-user cap and any Google policy action are per project, and the live sign-in is the one Google integration that works for everyone today. Google also asks for separate clients per workflow. Cost: one more brand verification and one more consent screen to keep current. *Alternative:* a second client in `agentmux-prod`; faster by a few days, and a rejected Calendar or Gmail review sits next to sign-in.

**D2. The id is a constant; the secret is injected at build.**
- `config_for("google").client_id` is the new client's id (a public identifier, in the public repository).
- The secret comes from the release pipeline's secret store into the build through an environment variable (`option_env!`, so a build without it simply has no built-in Google), and never from the repository: GitHub push protection recognizes Google client secrets, and the repository is public. **AgentY owns the pipeline change** (a named secret in the pipeline's own store, not handled by hand).
- **Dev builds** use the same environment variables at run time (as GitHub's does), so `task dev` and the `agentmux-dev` client keep working for the tiers not yet verified.
- **Test first:** before the secret is wired, the build's first PR runs one real code exchange and one refresh **without** the secret against the new client. If Google accepts, the secret isn't shipped at all and `requires_secret` becomes false for the built-in client. If it refuses (the code's comment says it will), the secret ships.
- **Rotation:** the console lets a client hold more than one secret. A rotation adds the new one, ships it, and disables the old after the old builds are gone; a build with a disabled secret says "update AgentMux" (§9).
- **Refresh** for a built-in account uses the built-in secret, not the blob's.

**D3. Ask for the union, every time.** Turning a service on, or reconnecting, always requests every scope the person has on plus the new one, whether or not `include_granted_scopes` works on installed apps (§2.3). The granted-scope read-back (built, G0a) is unchanged. A live test settles it, and the union is correct either way.

**D4. Scopes are what a shipped feature uses.** Stage by stage (§8): Drive `drive.file`; Calendar's three read scopes; later `calendar.events` when G1b ships; Gmail `gmail.readonly` and `gmail.compose`. `gmail.modify` (labelling and trashing) is the Google spec's open question 7: it widens the restricted request, so **recommended: leave `label_thread` and `trash_thread` out of the first Gmail submission**, and add them later with their own justification.

**D5. BYO and workspace-internal clients:** recommended: **hide them**. The Connect panel shows no client fields; the saved-client plumbing and the environment override stay in srv as the dev and test fallback until all three tiers are verified, then a cleanup PR deletes them. Nothing user-facing mentions a client.

**D6. The build only asks for what is verified** (§8): a capability list per build says which services the built-in client may request. It is a compile-time list plus the same environment override for dev builds.

**D7. An agent may hold several Google accounts, and a grant is all it needs (added 2026-11-01, for dogfooding several mailboxes).** Today an agent is bound to one Google account (an identity link set in the window) and the `account` argument of a call only checks that it is that one. That cannot read three mailboxes from one agent, and the binding is itself a manual step. The grants table already keys `(agent, account, service)`. So: a call names the account by email, and srv resolves it among the accounts that agent holds a grant on for that service (the bound account counts as one); with no `account` and one candidate it uses that one; with several it answers "name one of: a, b, c". `grant` needs no binding first. Tools are listed when any account has the service. Nothing else about grants changes: levels, the host-window approval for sends, and the per-call log (which already names the account).

## 5. The verification path, who does what

### 5.1 Setup of the new project (agents, in a Browser pane; the operator only signs in)

1. Create `agentmux-connectors` in the `agentmux` folder; no billing.
2. Enable the Calendar, Drive and Gmail APIs.
3. Google Auth Platform: Branding (name "AgentMux", logo, homepage, privacy, terms, authorized domain `agentmux.ai`, support `support@agentmux.ai`, developer contact), Audience External, and **publish**: brand verification starts (2-3 business days).
4. Data Access: declare only the scopes of the stage being submitted (D4).
5. Clients: Desktop app "AgentMux desktop", the AI-agent box ticked. The client id goes into the repository; the secret goes to AgentY's pipeline step (D2) and nowhere else.
6. Record it in the corp record and the infrastructure report (both private).

### 5.2 The three submissions

- **Submission A (after brand verification):** Drive `drive.file` is non-sensitive and needs nothing more: shipped as soon as the brand is verified. **Calendar** (sensitive) is submitted with the justifications and video of §5.4.
- **Submission B (Gmail restricted):** sent **after A**, so that Gmail's longer review and the security-assessment question can't hold Calendar. It carries Limited Use, the data-flow statement, and the question of §6.
- A re-submission after a rejection resubmits everything, so each submission is checked against Google's list of rejection causes first (§5.6).

### 5.3 Texts that go in the privacy page (agentmux.ai/privacy)

A new "Google" section, which AgentY's landing lane publishes: the Limited Use sentence of §2.4; what is accessed (the scopes, in words); that mail, events and files go from Google to the person's own computer and from there to the model provider the person's agent already uses; that AgentMux runs no server in that path and keeps none of it; that nothing is used to train a model; how to disconnect and revoke. Written by an agent from this spec; the founder's review and publish.

### 5.4 Per-scope justifications (first drafts, one sentence each; the submission has the final text)

- `drive.file`: lets the person pick the files an AI agent on their own computer may read, through Google's file picker, with no access to anything else in their Drive.
- `calendar.calendarlist.readonly`: lets the person's agent see which calendars they have, to pick the right one.
- `calendar.events.readonly`: lets the person's agent read their events to answer "what is on my calendar".
- `calendar.events.freebusy`: lets the person's agent find a free time without reading event details.
- `gmail.readonly`: lets the person's agent search and read their own mail threads on request. A narrower scope can't: `gmail.metadata` omits the bodies, which are what the user asks about.
- `gmail.compose`: lets the person's agent create a draft, and send it only after the person approves it in AgentMux's own window.

**How the submission describes the model provider** (Gmail and Drive content; Calendar's too):
- *Where the data goes:* from Google to the person's own computer, then, only when the person asks an agent about it, the text that action returned goes to the model provider that agent already runs on (for example Anthropic), under the person's own account and that provider's terms. AgentMux chooses no provider, holds no copy and operates no server in the path.
- *Consent:* the person grants Gmail to each agent separately, and the grant screen names the model provider each agent uses and says in one sentence that mail they ask about is sent to it. That screen is in the demo video.
- *What is sent:* only what a single action returned (a thread the person asked about, size-capped: a message is cut at 20,000 characters, a thread at 30 messages and 120,000), never the mailbox in bulk.
- *No training and no retention by us:* AgentMux trains no model and stores no mail. **Each provider's own no-training and retention terms are quoted from the provider's published terms before submission**; this spec does not state them, since they were not checked.
- *Prompt injection:* the measures of §2.4.
- *The question:* the submission asks Google whether sending a user's own mail to the provider the user's agent uses, at the user's request, is within the "user-directed appropriate use case", and says AgentMux will show the provider and require the consent above either way. If Google treats the provider as a third-party server, the data path the assessment reviews includes it (§6).

The **demo video**: an agent drives the app and a screen recorder records it; the consent screen with the client id in the address bar; each feature. Who can start the recording: an agent if a recorder can be launched from `Terminal` (checked in the build); else the operator presses record. **Upload to YouTube as Unlisted needs the operator's sign-in**; nothing else in the video needs a person.

### 5.5 What is a person's

Signing in to the Cloud console and Search Console; signing in to YouTube for the upload; the Google consent in every live test; **pressing Submit** on a verification (it is a statement to Google in AgentMux Corp's name); and anything that costs money (§6). Everything else is an agent's.

### 5.6 Rejection causes checked before each submission

From Google's pages: scopes broader than a shipped feature; scopes in the code that differ from the consent screen; a homepage that is only a login page; a privacy policy on another domain; the domain not verified; test redirect URIs or test environments in a production client; a video missing the consent screen, the client id in the address bar, or a scope's feature. The checklist is a test (§9).

## 6. The Gmail decision (for the operator)

**What Google says, and what is known about how it applies.**
- Google's restricted-scope page conditions the annual assessment on server storage or transmission: "If you store or transmit restricted scope data on servers, then you need to complete a security assessment", and "from or through a third-party server".
- Google's Help Center security-assessment page has no such condition: "applications requesting access to restricted scopes must undergo an annual security assessment".
- **A local desktop mail app was told this year that local doesn't exempt it.** The Flectar Mail issue tracker (github.com/flectar/mail/issues/114, 2026-10-22) quotes Google's reply after about two months: "Every app that requests access to restricted scope is now required to go through a security assessment validated by an ADA-authorized lab. This includes local client applications." That is a developer's report of Google's reply, not a Google page; it is the best evidence available and it matches the Help Center wording.
- **The model provider may count as the third-party server** (§2.4), so even the server-qualified wording could reach AgentMux's data path, because the mail goes on to the provider the user's agent uses.

**So: plan on the assessment.** Ask Google in Submission B anyway (free, and it settles the model-provider question in writing), but the schedule and the budget assume the assessment.

**What it costs and takes** (third-party price lists; Google charges nothing and sets no price, "agreed between the developer and the assessor"):
- **Tier 2:** TAC Security (the assessor Google steers to) lists $540 a year, premium options up to $1,800; turnaround 1-3 weeks. Leviathan lists $800-$1,200. Bishop Fox $1,500 and up, NCC Group $1,200 and up, NetSentries $900-$1,500, Prescient $1,000 and up. Sources: switchlabs.dev's provider comparison (undated) and bright-softwares.com's article, which gives $540 to $1,800 a year through TAC Security and calls the old $50,000 to $75,000 figure wrong (published 2026-04-27, updated 2026-10-30).
- **Tier 3** (if Google assigns it): TAC $4,500; others $5,000-$8,000; 2-6 weeks.
- **The $15K-75K figure in earlier notes is out of date** and is dropped.
- **Which tier Google assigns is not ours to choose:** Google says it is calculated from the sensitivity of the data, the number of users per data type and internal risk indicators. A mail-reading app with many users could be assigned Tier 3. The quote asks the assessor which tier, with the data flow of §5.4.
- **Annual:** the assessment recurs every 12 months from the Letter of Assessment, and a changed app (a new scope) may need a new one.
- The figures are a range from sellers, not a quote. **Nothing is bought, and no assessor is contacted for a price that commits us, before the operator says.**

**Options**
1. **Submit the restricted application and take the assessment** (recommended). Budget about $540-$1,800 a year for Tier 2 (up to about $5,000-$8,000 if Tier 3), 1-6 weeks for the assessment, and about 6 weeks for Google's review, which can overlap.
2. **Ask Google first and hold Gmail until it answers.** Free, but Google's wording and the Flectar answer say it will say yes; the cost is the weeks.
3. **Offer Gmail "send only"** (`gmail.send`, sensitive, in Submission A with Calendar): an agent can send mail after the host window's approval, but can't read or draft in Gmail. Useful for some work, not the reads that make Gmail valuable. No assessment.
4. **No Gmail on AgentMux's own client**; Drive and Calendar only. A Workspace admin's own internal app is the only Gmail path, and it is not a consumer option.

The timeline for 1: Submission B about **6 weeks** from sending; the assessment (1-6 weeks) starts when the assessor is engaged and can run in parallel, so Gmail is about **6 to 10 weeks** after the operator's go, the assessment's price being the only spend.

## 7. What carries over from the BYO spec, what doesn't

**Carries over, unchanged in design** (built after this spec is approved, one PR each, §10):
- the **`Google` tool** (always listed): `status`, `connect`, `connect_status`, `cancel`, `grant`, `ungrant`, `disconnect`;
- **grants by agents**: decided by the operator, "yes, agents can grant". Any agent may grant and ungrant, recorded as that agent, shown in the account's log and in a non-blocking window notice. Sends, shares, invites and deletes still wait in the host window, which no agent can answer;
- **status that says why** (no account; an account but no grant; reconnect needed since when; blocked by the organization; a service the build doesn't offer yet);
- **the tool appearing without a relaunch** (`notifications/tools/list_changed`, with `Google` `call` as the fallback for a client that ignores it);
- **opening Google's sign-in for the person**: the agent opens the URL in a Browser pane and hands off, or the host opens the system browser (a container agent's pane is Incognito);
- **the edge cases** (§9), except the client-file ones;
- **another machine:** `status` and `grant` with `host`; `connect` stays local, since the sign-in is a person's, at that machine.

**Dropped:** capturing the client file into the Vault, the `google-oauth-client` kind, `Vault delete`, the `made_by` record, the shared client-file parser, `connect {client}`, and Cloud console automation *for users*. (The same console work is done once, by an agent, for AgentMux's own project, §5.1.) The Vault code started for this is not built.

**Closed:** #6092 (the "Import the JSON you downloaded" button): BYO-only.

## 8. What a user sees before each tier is verified

Public builds only request a service once its tier is approved, so a user never sees an unverified screen or counts toward the 100-user cap.

| Stage | Public build offers | The Connect panel shows for the rest |
|---|---|---|
| **0 (today)** | Nothing: no built-in Google client | "Google isn't available in this build yet" |
| **1: connectors project brand-verified, client built in** | Sign-in, **Drive** (the file picker) | Calendar and Gmail: "Coming soon" with no switch |
| **2: Calendar approved by Google** | + **Calendar** read (writes follow with G1b) | Gmail: "Coming soon" |
| **3: Gmail approved** (and assessed) | + **Gmail** (reads, drafts, send with approval) | none |
| **Internal builds** (the operator, the agents' humans, up to 100 users) | Everything, through the `agentmux-dev` client (published, unverified; no 7-day expiry), set by the environment override | none |

Sign-in with AgentMux's own Google login (`agentmux-prod`) is unchanged at every stage.

## 9. Edge cases that change or are new

The cases of the BYO spec (§7 there) that don't involve a client file apply as written there: denied or partly granted consent, a callback that times out or a port that fails to bind (the OS picks the port, so only the bind can fail), a second connect while one waits, a revoked or expired refresh token, a different Google account (refused: "you signed in as X, but this account is Y"), two agents granted, a revoke or disconnect mid-call (the grant is checked again before the result returns), a container agent, an instance on another machine, offline, a locked keychain, and a workspace admin blocking the app. New or changed for Phase B:

| Situation | Behaviour | Test |
|---|---|---|
| **The 100-user cap is reached** for a tier not yet verified | Cannot happen to a public build (§8). An internal build that reaches it says "Google has turned off new sign-ins for this app's unverified scopes" | Mock the Google error |
| **The built-in secret was rotated and this build still has the old one** | `invalid_client` at exchange or refresh: "This version of AgentMux can no longer sign in to Google. Update AgentMux." The account is not marked broken | Mock `invalid_client` |
| **A build has no built-in Google** (no secret injected, a fork) | The Connect panel says so plainly and offers nothing to type; the environment override still works for a developer | Build without the variable |
| **Testing-mode expiry** | Applies only to internal builds on `agentmux-dev`; `status` says "this connection comes from a test project: it ends 7 days after consent" | Unit |
| **The person's account is over Google's live-token limit** (100 refresh tokens per account per client; the oldest is dropped without warning) | The oldest connection shows "Reconnect needed"; reconnecting works | Mock `invalid_grant` |
| **The person changed their Google password** (invalidates refresh tokens that hold Gmail scopes) | "Reconnect needed" with that reason when Google says so | Mock |
| **Incremental scopes don't carry over on an installed app** | D3 asks for the union every time, so the new token always holds everything on | Live test; unit on the request |
| **A user disconnects in Google's own settings** | The next refresh returns `invalid_grant`: "Reconnect needed"; grants kept | Mock |
| **A scope is declined on the consent screen** | The service shows "Partly granted: <what's missing>" (built) and its tools are limited to what was granted | Built |
| **Google changes a tier** (a scope's class changes) | The build's capability list is data (D6): one edit, no schema change | Unit |

## 10. Build plan

One PR each, in this order, with tests and a changeset; the first live pass runs after row 3.

| PR | Contents | Depends on |
|---|---|---|
| 0 (no code) | The connectors project, Branding published, the Desktop client (§5.1); the corp record. Agents drive the console; the operator signs in | Operator's go |
| 1 | The id constant; the **no-secret exchange and refresh test** (D2); `resolve_client`, refresh and the build-time and run-time secret variables; the capability list (D6); Gmail refused as "not available yet" instead of "bring your own" | 0 |
| 2 | AgentY: the release pipeline injects the secret | 1 |
| 3 | The Connect panel without client fields (D5); the staged service rows of §8 | 1 |
| 4 | `Google` `status`, `connect`, `connect_status`, `cancel`, with the union request (D3) and the sentences of §9 | 1 |
| 1b | **Early, for dogfooding (D7):** `Google` `status`, `grant`, `ungrant` (agents turn services on themselves), several accounts per agent, no binding needed | 1 |
| 5 | `disconnect` and the window notice; the window's Services calls the same code; the in-flight re-check | 4 |
| 6 | `list_changed` and the `Google` `call` fallback | 5 |
| 7 | Remote `google.status` and `google.grant` | 5 |
| 8 | The privacy page's Google section (AgentY's landing lane); Submission A (Calendar) assembled; the rejection-causes checklist as a test | 3, brand verified |
| 9 | Submission B (Gmail) assembled and the question of §6 | 8 |
| 10 | Cleanup: delete the BYO and workspace-internal client code once all three tiers are verified | stage 3 |

The live pass on `agentmux-dev` (Calendar, Drive, Gmail with a person for the sign-in) runs right after PR 5; it settles the incremental-authorization question (D3), the Drive picker on the new client, and `drafts.send` with `message.raw`, all untested against a real account. It is recorded in the Google spec's §8.

## 11. For the operator

1. **Approve D1** (a separate `agentmux-connectors` project) or choose a second client in `agentmux-prod`.
2. **Gmail (§6):** plan on the assessment (about $540-$1,800 a year for Tier 2, more if Google assigns Tier 3) and approve getting a quote, or choose another option. Nothing is spent before you say.
3. **Go for PR 0**: I create the project and publish its branding (2-3 business days); I need you signed in to the Cloud console for that. No cost.
4. **Who records and uploads the demo video:** I drive the app; you sign in to YouTube.
