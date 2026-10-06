# Spec: The new-agent Name field starts empty, with ghost text

**Status:** implemented (#4397)
**Date:** 2026-10-06
**Author:** Camper

## Ask

When you create an agent from the agent picker, the **Name** field must start
empty and show ghost text ("Choose a name"), for every provider. Today it is
pre-filled with the provider's template name, so agents pile up as `Claude`,
`Claude2`, `Claude3`, which says nothing about what each one is for.

## What happens today

There is one path that creates a new agent, and every provider goes through it:

1. Agent picker → a card under **New Agent** (Claude Code, Codex, Gemini, …).
2. `AgentPicker.tsx` `openCreateFromTemplateModal(template)`. The install and
   prereq gates route here too once they finish.
3. `AgentCreateFromTemplateModal.tsx` (**Create new agent from {template}**)
   collects Name, Runtime, Model, Account and Bundle, then calls
   `agentdefcreatefromtemplate` and launches the new agent.

The Name field is initialised from the template:

```ts
// AgentCreateFromTemplateModal.tsx:75
const [name, setName] = createSignal(props.initialName ?? props.template.name);
```

and its placeholder is the template name too (`placeholder={props.template.name}`,
line 335). No caller passes `initialName`, so the field always opens holding the
template's name, and **Create** is enabled immediately.

The backend rejects a name that a user-owned agent already has, case-insensitively
(`server/agent_handlers/template.rs`, "an agent named … already exists"). So the
first agent is called "Claude"; the next attempt errors, and the user gets past
the error by typing a digit. That is where `Claude2` and `Claude3` come from.

### Why it wasn't already fixed

#3097 ("stop autofilling the agent name from the model",
`SPEC_AGENT_LAUNCH_NAME_NO_AUTOFILL_2026_09_08.md`) made exactly this change,
but only in `AgentLaunchModal.tsx`: that field now opens empty with the
placeholder "Descriptive nickname". The create-from-template modal arrived with
the two-tier picker (#1011) and kept its own pre-fill, and it is now the modal
every new agent goes through. `SPEC_DEFAULT_AGENT_NAME.md` (issue #780) proposed
the pre-fill in the first place; #3097 reversed it.

## Change

### 1. Empty field, ghost text (`AgentCreateFromTemplateModal.tsx`)

- Initial value: `props.initialName ?? ""`. `initialName` stays, for a future
  caller with a real reason to suggest a name; nothing passes it today.
- Placeholder: **`Choose a name`**. The same for every provider, never the
  template or provider name. The provider is already in the modal title.
- A hint under the field, matching the Launch modal's hint style:
  **"Name it for its job, like Reviewer or Docs writer, so you can tell your
  agents apart."**
- Keep `autofocus`, so the user can start typing straight away.
- **Create** stays disabled until the trimmed name is non-empty. `canSubmit()`
  already requires that; with an empty initial value it now takes effect.
  Ctrl/Cmd+Enter goes through the same `canSubmit()` gate.

### 2. One wording for both name fields

Use the same placeholder in `AgentLaunchModal.tsx`: change "Descriptive
nickname" to "Choose a name", so both places you name an agent read alike. The
Launch modal's existing hint ("So you can tell it apart from other agents.
1–64 characters.") stays.

### 3. A readable duplicate-name error

The server's error text currently reaches the modal raw, prefixed with the RPC
name: `agentdefcreatefromtemplate: an agent named "X" already exists`. Show it
as **"You already have an agent named X. Choose another name."** Do this in the
modal by recognising the duplicate case; the server's message and behaviour stay
as they are. The modal stays open with the typed name kept, so the user can edit
it.

## Out of scope

- Generating names, or numbering them (`Claude 2`). That is exactly what this
  removes.
- Renaming existing agents called `Claude2` and similar. They are the user's
  own names, and stay.
- The two fields' different maximum lengths (200 here, 64 in the Launch modal).
  This is a real inconsistency, but a separate change.

## Tests

`AgentCreateFromTemplateModal.test.tsx`:

- Replace "defaults the name field to the template name" with: the field opens
  with value `""`, placeholder `"Choose a name"`, and **Create** disabled.
- Run that check for at least two templates (Claude Code and Codex), so the
  placeholder is shown not to vary by provider.
- Tests that submitted without typing relied on the old pre-fill; they pass
  `initialName` instead.
- Typing a name enables **Create**; whitespace only does not.
- Ctrl+Enter with an empty field does not call `onSubmit`.
- A rejected `onSubmit` carrying the duplicate-name error shows the friendly
  message and keeps the typed value.

`AgentLaunchModal` tests: update any assertion on the old placeholder.

**Manual check, in a `task dev` build:** picker → New Agent → each installed
provider. The field is empty with ghost text, **Create** is disabled until you
type, and creating a second agent with an existing name shows the friendly
error.

## Size

One component, the Launch modal's placeholder string, and their tests: about
40 lines of change, frontend only. No migration, and no server change.
