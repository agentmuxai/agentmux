# Fake forms

A page of forms and controls for the Browser tool's `key` action with Enter and
Space on a grant that carries the `enter_space` power
(`docs/specs/SPEC_BROWSER_WEB_TERMINAL_TYPING_2026_10_23.md` §5, §13).

    node test/fixtures/fake-form/server.mjs [port]     # default 8766

Every form's submit is caught and recorded, not sent: `window.__submits` holds
`{form, fields}` for each, `window.__clicks` the other effects.

| Focus on | Enter | Space |
|---|---|---|
| `#q` (search form) | clicks "Search", a submit: asks, or runs under a grant | types a space |
| `#holder` (pay form) | clicks "Pay now": a payment always asks | types a space |
| `#user`, `#pw` (login form) | clicks "Sign in"; the password is not typed into | types a space; refused in `#pw` |
| `#lonely` | refused: the form has no submit button | types a space |
| `#own` | refused: the field listens for keys itself | refused |
| `#deleg` (a form that listens for keys) | refused: the form listens for keys | refused |
| `#note` (textarea) | refused | types a space |
| `#loose` | refused: not in a form | types a space |
| `#plain` | clicks it | clicks it |
| `#agree` | refused: not in a form | toggles it |
| `#lnk` | follows it | refused |
| `#widget` | refused: a custom widget | refused |
