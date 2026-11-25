---
name: ui-confirm-panels
description: Use when adding or changing a confirm, prompt, approval or "are you sure" in the AgentMux UI (a delete confirmation, a permission request, an install or upgrade approval), or when you meet a modal. A plain confirm is a panel inside the element that asks; a window or tab modal is only for a whole window or tab; a pane modal is for a flow the person works through inside the pane, and an existing one changes only when a task names it.
---

# Confirms are panels; modals are for specific cases

A confirm, a prompt or an approval is a panel inside the UI element that asks (the pane, popover, row, menu or card the person just acted in).

## New UI

- **A plain "are you sure?" is a panel inside the element that asks.** Not a modal over the window, the tab or the pane.
- **Keep the confirm.** The rule removes the modal, not the question. Don't drop a confirm to get rid of its modal.
- **A window or tab modal is only for a question about a whole window or a whole tab:** closing a window, closing a tab with running work, quitting.
- **A pane modal is allowed for a flow the person works through inside the pane:** a sign-in that needs a code, a form with several steps. It is not for a single yes-or-no question; that is a panel.
- **When unsure which one a screen needs, ask the UI kit's owner** before building it.

## Modals that already exist

- **Don't convert pane modals in a sweep, and don't convert one you meet in passing.** Whether an existing pane modal changes is decided one at a time: it changes only when a task names it. Report a modal you think should change to the UI kit's owner, who files that task.
- **A window or tab modal about something smaller than a window or tab** still belongs to the UI kit's scan and its order of conversion (in that kit's spec). Don't rework one in passing; report one the scan missed to the UI kit's owner.
- **Don't remove a pane modal that serves a flow** (a sign-in with a code, for one) because it is a modal.

## Always

- **Approvals stay a person's act wherever they move.** Moving an approval surface (a credential use, a widget install, an upgrade, a desktop request) is decided per surface, with a security read. Never weaken one while moving it.
- **Not confirms:** the operating system's own file pickers, lightboxes, the command palette and full forms.
- Rule M1 ("Inline before modal") in `docs/specs/SPEC_UI_LINE_STYLE_COMPONENT_SYSTEM_2026_10_05.md` still allows a modal for a destructive confirmation until that spec is rewritten to match this rule.
