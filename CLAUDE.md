# AgentMux

Instructions for anyone, human or agent, changing this repository. Release steps are in `.claude/skills/release`, and how to run the app to check a change is in `.claude/skills/run`.

## UI: no window modals

A confirm, a prompt or an approval is a panel inside the UI element that asks (the pane, popover, row, menu or card the person just acted in). It is never a modal over the window, the tab or the pane.

- **Keep the confirm.** The rule removes the modal, not the question. Don't drop a confirm to get rid of its modal.
- **A modal is only for a question about a whole window or a whole tab:** closing a window, closing a tab with running work, quitting. Any other modal needs the project owner's approval, asked through the UI kit's owner.
- **Touching a screen that still has a modal confirm?** Don't rework it in passing: the scan and the order of conversion are in the UI kit's spec. Report a modal the scan missed to the UI kit's owner.
- **Approvals stay a person's act wherever they move.** Moving an approval surface (a credential use, a widget install, an upgrade, a desktop request) is decided per surface, with a security read. Never weaken one while moving it.
- **Not confirms:** the operating system's own file pickers, lightboxes, the command palette and full forms.
- Rule M1 ("Inline before modal") in `docs/specs/SPEC_UI_LINE_STYLE_COMPONENT_SYSTEM_2026_10_05.md` still allows a modal for a destructive confirmation until that spec is rewritten to match this rule.
