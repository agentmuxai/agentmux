# SPEC: Vault card entries hold a full card (number, expiry month and year, CVC, name, billing ZIP) that people and agents can store and fill

**Status:** active. Approved by the operator on 2026-11-08 (relayed by AgentY): all seven decisions in section 3 as recommended. C1 is built (see "As built (C1)" at the end). Spec task #29012, build #29013. The operator's words (relayed by AgentY, verbatim): "credit cards need to save expiration and cvc. also any other edge cases, why would that be left out?", "lets store it" (the CVC), and "agents can also store and use them too".
**Author:** Agent2-6 (Vault). **Extends:** `SPEC_VAULT_PANE_AGENTS_USE_SECRETS_UNSEEN_2026_10_19.md` (the Vault spec), `SPEC_VAULT_GENERATE_CAPTURE_AND_BROWSER_SIGNINS_2026_10_28.md` (capture) and `SPEC_VAULT_PARITY_ENTRY_HISTORY_REASONS_AND_IMPORT_2026_11_06.md` (reasons and history).
**Verified on:** `main` at `0cbd5fdbe` (2026-11-08).

## 1. Summary

A `card` entry already has four sealed parts (`number`, `expiry`, `cvc`, `name`), but only half of a checkout can use them. The Vault pane's form takes them (the expiry as free text "MM/YY"); a value an agent asks the person to type (`Browser` `request_value`) and a value an agent captures (`Browser` `capture`, `Terminal` `capture`) are one part at a time, so a card made that way holds a number and nothing else; the fill puts one stored part into one text field, with one approval per field; and nothing handles a month or year `<select>`, a 4-digit security code, a billing ZIP, an expired card or a second card.

This spec makes a card a first-class entry:

1. **A schema** with the expiry month and year stored apart, a billing ZIP, and a card brand worked out when it is saved (§4).
2. **One way to make a card, by anyone:** the person in the pane, the person typing it into an agent's request, an agent capturing it from a page or a command's output (§5). Nothing about storing a card is limited to the person.
3. **One approval to fill a whole card**, a picker for which card, and a fill that copes with how checkouts really are (§6).
4. **A security posture** for a Vault that now holds a full card and its CVC (§7).

Agents, native and container alike, store and fill cards the same way the person does and never see a value. There is no operator-only gate anywhere in this spec (§8).

## 2. What exists today (from the code)

- `kinds.rs`: `card` has sealed parts `number` (the main one: its last four characters are the pane's tail), `expiry`, `cvc` and `name`, and plain fields `owner` and `notes`. It has no `expires` field (that is for API keys, logins, notes and IDs), so the expiry reminders (`expiry.rs`) never see a card.
- The pane's form (`vault-kinds.ts`) shows the four parts, the expiry with the placeholder "MM/YY" and no format check. Imports parse a card's expiry into "MM/YYYY" (`import/shared.rs`), so entries made by import and entries typed by hand disagree about the format.
- Fill (`slots.rs`, `crates/cef/src/browser_api/vault_slots.rs`, `server/browser_vault_fill.rs`): each part maps to a slot (`card-number`, `card-expiry`, `card-cvc`, `card-name`); a form field is matched to a slot by its `autocomplete` token or by words in its name, id or placeholder; srv asks per field (an "Always on this site" grant is per part and site); a field that is not a text input is refused (`takes_text`), so a `<select>` is never filled; the value is typed as one string, whatever shape the field wants. A field in a cross-origin frame is reached through its frame's own session, and its origin (the frame's) is what the person is asked about.
- Creating a card: `Vault` `request` opens the pane's full form for the person. `Browser` `request_value` asks for **one** value, typed into a password-style box. `capture` (page or command output) targets one part of one entry: a new entry must be made from its main part (`number`), and two rules naming the same new entry are refused, so a card can't be built from several captured values.
- Redaction (`scrub.rs`): the values an approved command was given are scrubbed from its output in their plain, base64, percent, hex and JSON forms, but only values of **4 bytes or more** (`MIN_LEN`). A 3-digit CVC would never be scrubbed.
- Nothing in the repository checks a card number's checksum (Luhn), names its brand, or knows an American Express code has four digits.
- A card is never used on silence: `goes_ahead_on_silence` is false for it, and the approval card says so.

## 3. Decisions to take (recommended answers first)

| # | Question | Recommended | Why |
|---|---|---|---|
| 1 | Store the CVC? | **Yes, sealed, with the rest.** *Decided by the operator: "lets store it".* | It is the same protection as the number; asking each time would make an agent's checkout impossible when the person is away. |
| 2 | A number that fails the Luhn check | **Warn, don't block.** The form says "this number doesn't look right" and still saves. | A typo is the common case, but gift cards, test cards and some local schemes exist; a block would be the Vault deciding for the person. |
| 3 | An expired card | **Warn on the card and ask every time**: an "Always" grant never covers an expired card; the fill still happens on Allow once. | An expired card is almost always a mistake, never dangerous; refusing outright would block a form that checks only the format. |
| 4 | Approvals when filling a card | **One approval for the whole card**, naming the parts, with Always for this agent on this site. | Today a checkout is five approvals. The agent still asks for a card, never for a part. |
| 5 | Which card does an agent fill? | **The person's pick**: the card the agent names, else the Vault's default card, else the approval card asks the person to choose. The agent never lists numbers. | Which card to pay with is the person's call; the picker keeps it theirs. |
| 6 | Billing address | **ZIP / postal code only** now (what address checks need most). A full address is a later addition. | Keeps this to what the operator asked for. |
| 7 | The CVC and a command's output | **Scrub a 3-digit code as a whole token** (not inside a longer run of digits), and the longer parts as today. | `MIN_LEN` would otherwise leave it in the output; a standalone "123" turning into `[vault:…]` in the output of a command the person approved for a card is an acceptable price. |

## 4. The card

**Sealed parts** of a `card` entry (all sealed, none returned to an agent):

| Part | Holds | Notes |
|---|---|---|
| `number` | digits only, no spaces or dashes | Main part. Grouped for display by brand (4-4-4-4; Amex 4-6-5). |
| `exp_month` | `01` to `12` | Replaces the month of `expiry`. |
| `exp_year` | four digits, `2030` | A two-digit year typed anywhere becomes `20xx`. |
| `cvc` | 3 digits; 4 for American Express | Optional: a card without one is still a card (a fill leaves that field for the person). |
| `name` | the name on the card | As today. |
| `zip` | the billing postal code, any characters a postal code uses | New. Optional. |

**Plain fields:** `owner`, `notes` as today; `brand` (`visa`, `mastercard`, `amex`, `discover`, `diners`, `jcb`, `unionpay`, or empty), worked out from the number's first digits whenever it is saved; and `default` (`yes` on at most one card per Vault: setting it clears it from the other). The brand and default are no secret, so the pane, the approval card and an agent's `Vault list` (which gains `brand` for a card) can show them without opening the seal.

**The old `expiry` part** (a "MM/YY" or "MM/YYYY" string) stays readable: when `exp_month` and `exp_year` are missing the fill and the pane read them from it, and a save from the form writes the two new parts and drops `expiry`. No bulk migration, nothing rewritten until the person or an agent saves the entry. A `Terminal` secret can still name `<id>.expiry`: it is derived as "MM/YY" (and `<id>.expiry_long` as "MM/YYYY") from the two parts.

**A pure module** (`backend/vault/card.rs`): `brand(number)` from the usual ranges (Visa 4; Mastercard 51 to 55 and 2221 to 2720; Amex 34, 37; Discover 6011, 644 to 649, 65; Diners 300 to 305, 36, 38; JCB 3528 to 3589; UnionPay 62), `luhn(number)`, `group(number)`, `cvc_length(brand)`, `normalize_number`, `normalize_expiry` (any of "3/27", "03/27", "03/2027", "0327", "03-2027" to month and year), `expired(month, year, today)` (expired from the first day of the month after), and the form-shape formatting of §6. Each has tests with invented numbers (the standard test numbers, none a real card).

**Reminders:** a card's expiry date (the last day of `exp_year`/`exp_month`) feeds the existing reminder schedule (two weeks, three days, on the day) as a notice to the person; unlike an API key it files no rotation task for an agent.

## 5. Making a card: the person and agents alike

Every route is in the container route list and works for native and container agents. Each use asks the person as any card use does, and an agent gives a reason (H2).

1. **The Vault pane.** One card form: number (grouped as it is typed, the brand's badge beside it, a Luhn warning), expiry as a month select and a year select (the current year to twenty years on, and a typed year for anything else), security code (the label and the length follow the brand: "CVC" with 3, "CID" with 4), name on card, billing ZIP, owner, notes, and a "default card" switch. Editing shows each sealed part hidden until Reveal, as now.
2. **`Vault request` (kind `card`)** opens that form for the person, as now; the form is the new one.
3. **`Browser request_value` (kind `card`)** shows the person the whole card form in the agent's card (number, expiry, code, name, ZIP, and "Save to the Vault as <label>"), not one password box. The value goes to the fill and, if saved, into one entry; the agent gets "filled" or the entry's id, never a value.
4. **`Browser capture` for a card** (the agent reads a card from a page it is on: a virtual-card page, a bank's "show card details"): `capture` takes a `card` object, `{ number, exp, exp_month, exp_year, cvc, name, zip }`, each a reference to an element on the page; the values are read in srv, normalised (§4), and made into **one** entry. The page's text is never returned.
5. **`Terminal` `capture` for a card** (a command prints the card as JSON, as a virtual-card API does): capture rules gain a `parts` form for one new entry, `{ into: { kind: "card", label, vault }, number: "$.pan", exp: "$.expiry", cvc: "$.cvv", name: "$.holder", zip: "$.zip" }`, using the capture paths that exist today; the card is stored in one transaction and the output shows `[vault:<id>]` in place of each. (The rule that two captures may not name one new entry stays for single-part rules.)
6. **File imports** keep working, and now also write `exp_month`, `exp_year` and `brand`, and read a billing ZIP where the file has one.
7. **`Vault` `update`** (H4) can change a card's label, `owner`, `notes` and the `default` flag; it never changes a part (that is a capture, a request or the pane).

An agent that stores a card is recorded as the actor of its `created` or `captured` line, with its reason.

## 6. Filling a card

**One approval, a card picker, one fill.** `Browser` `fill` with `vault: "<card id>"` (or `vault: "card"`, meaning the default) on a form that has card fields fills every part the form asks for that the card holds, after **one** approval:

> *<Agent> wants to fill a card into <site>* — **Visa •••• 4242**, exp 03/27 — number, expiry, security code, name, ZIP. *<Agent> says: "…"*. [Card ▾] Deny · Allow once · Always for <agent> on <site>

The picker lists the person's cards in the Vaults the agent may use (brand, last four, expiry, default marked); the agent's choice, the default or the person's pick is the preselected one. An agent that names no card and finds no default gets the picker with nothing preselected. A card that has expired shows "expired 03/2026" in the card, an "Always" grant never covers it, and silence denies as always. The old per-part `fill` (`vault: "<id>.<part>"`) stays, for a form that needs only one field.

**Field matching** (the host's classifier grows the slots): `card-number`, `card-expiry` (a single field), `card-exp-month`, `card-exp-year`, `card-cvc`, `card-name`, `card-zip` (an `autocomplete` of `cc-number`, `cc-exp`, `cc-exp-month`, `cc-exp-year`, `cc-csc`, `cc-name`, `postal-code` inside a billing section, or the usual words in a field's name, id, label or placeholder). Password and one-time-code fields are never filled, as now.

**Value shape per field:**

- **Expiry in one field:** `MM/YY` by default; `MM/YYYY` when the placeholder, label or `maxlength` says so (a `maxlength` of 7, "YYYY" in the placeholder); `MMYY` or `MMYYYY` when `maxlength` is 4 or 6 with no separator in the placeholder. A `pattern` is honoured when it names one.
- **Month and year as separate fields:** text inputs get `03` and `2027` (or `27` for a `maxlength` of 2). A `<select>` is filled by choosing the option whose value or label matches the month (`3`, `03`, `March`, `Mar`) or the year (`2027`, `27`); the host sets the choice and fires `input` and `change`; no value is returned. (Today only text inputs are filled.)
- **Number:** digits only; a field whose `maxlength` is 19 or more and that groups as you type gets the grouped form.
- **Security code:** 3 digits, or 4 for American Express; a field whose `maxlength` is smaller than the code is left empty and said so ("this field takes 3 digits and the card's code has 4") rather than cut.
- **ZIP:** as stored; left empty when the card has none.
- **A part the card lacks** (no CVC, no ZIP) is left for the person; the answer lists the parts filled and the parts the form wanted that the card doesn't hold, names only.

**Payment frames.** A card field in a cross-origin frame (Stripe Elements, Braintree, Adyen, a bank's hosted field) is filled through that frame's own session, as a frame's field already is. The approval names both the frame's origin and the page's site ("card field from js.stripe.com on shop.example"); a grant is for that pair, so a page's own look-alike frame never inherits it. Some payment fields ignore a programmatic value (the page's validation never sees it). The fill then types the value as key events, character by character, in the frame (still without ever returning it); if the field still does not accept it, the fill says "the field didn't take the value" and the agent uses `Browser` `handoff` to let the person type that part. The agent can read the page's own validation message, as it can on any page.

**Agents never see a value:** the fill answers the parts filled and the field labels; a fill marks each field it filled (`data-agentmux-vault`) so a snapshot, a read-back and a screenshot treat it as secret (existing behaviour, kept for the new slots).

## 7. Security and redaction

- **At rest:** a card is sealed with the Vault key, as every entry is, in the one sealed value per entry; the new parts add no new storage. `brand`, `default`, the last four (the tail) and the expiry date (`expires`, below) are the only things readable without the key.
- **Never in plain text, anywhere:** a test registers the number, the CVC, the ZIP, the name and the expiry of a card used in the suite, then scans the application log, srv's log, the action log, the Audit rows, the history rows (and their `target` and `reason`), every response the new and changed routes give, every error, and a screenshot's text layer for them. None may appear. History lines name parts (`number`, `cvc`), never values.
- **Scrubbing:** every part value given to a command is registered with the scrubber in the forms it already uses. A value shorter than four bytes (the 3-digit CVC, a 2-digit year) is scrubbed as a whole token only (not inside a longer digit run), and never reported as scrubbed text; the others as today. A card's `Terminal` secrets are granted per part (`command:<agent>:<part>`), so a command given the number need not be given the CVC.
- **Agents:** `Vault list` gains `brand` for a card and nothing else; `Vault show` (H4) names the parts that are set, never their values; `Vault history` shows no value. The approval card shows the brand, the last four and the expiry to the person only.
- **The person's own actions** (Reveal, Copy) keep the OS confirmation. A file export keeps its own confirmation; a card is in it as any entry is.
- **Not a merchant:** AgentMux stores the person's own cards on their own machine and puts them in the page the person approved; it never sends a card anywhere else. Nothing here makes AgentMux a payment processor, and no claim of PCI scope is made.

## 8. Container-first, no operator-only gate

Every route this spec adds or changes (`fill` for a card, `request_value`, `capture`, `Terminal` `capture`, `Vault` `update`) is open to native and container agents like the rest of the Vault; a container agent's browser panes use their own profile, and the fill is the host's, as for any Vault fill. The approval is the person's, as for every use of a card; there is no check that needs the operator's words. An agent can store a card (§5) and fill one (§6); what it cannot do is read one.

## 9. Phases (one PR each, in this order)

| PR | What | Depends on |
|---|---|---|
| **C1** | `card.rs` (brand, Luhn, normalisation, expiry), the new parts and plain fields, the old `expiry` read-through, the pane's card form and list row (brand badge, last four, expiry, expired chip, default), reminders, imports writing the new parts; tests; mac and Linux look checks | Nothing |
| **C2** | The one-approval card fill: `Browser` `fill` for a whole card, the picker, the new slots, month and year selects, field-shape formatting, security-code length, ZIP, expired-card handling, payment frames (the key-event fallback); tests; look checks | C1 |
| **C3** | Agents store a card: `request_value` for a card, `capture` for a card from a page and from a command, the derived `Terminal` parts, `Vault list` `brand`, `update` of `default`; the 3-digit scrub; the no-plain-text scan | C1 |

C2 and C3 are independent once C1 is in.

## 10. Tests

- **`card.rs`:** the brand ranges, Luhn on the standard test numbers and on one-digit-off numbers, every accepted expiry spelling, `expired` on the month boundary, the CVC length per brand, grouping.
- **Fill:** a combined expiry in each shape (`MM/YY`, `MM/YYYY`, `MMYY`, by `maxlength` and placeholder), split text fields, month and year `<select>`s with each option style, a code field that takes fewer digits than the card's, a card with no CVC or ZIP, an expired card (the warning, no Always), a frame field (the pair in the approval, the key-event fallback, the handoff message), a form with a password field next to the card fields (never filled), one approval for the set, the picker's choices and the default.
- **Making a card:** the form (Luhn warning, brand badge, Amex four-digit code), `request_value` for a card (one entry, no value in the answer), `capture` from page references and from command JSON (one entry from several values, the output shows `[vault:<id>]`), the old `expiry` part still filling, a save from the form dropping it.
- **Leaks:** the scan of §7 over the whole flow; the 3-digit scrub (a standalone `123` scrubbed, `4123` left alone).
- **Pane:** the form and the list row at the sizes the pane uses, dark and light; macOS and Linux look checks for C1 and C2.

## 11. What this spec does not do

- A full billing address, a shipping address, a phone number: later, on the same parts.
- 3-D Secure and one-time codes: the person answers them (`handoff`).
- Wallets (Apple Pay, Google Pay), virtual-card issuing, tokenisation: not AgentMux's.
- Syncing cards between machines: the shared Vault spec (VS2, VS3) carries entries as records of its layer.
- Checking that a card works, or that a number belongs to a real account.

## As built (C1)

- **`backend/vault/card.rs`** is the pure module of section 4: `brand`, `luhn`, `group`, `cvc_length`, `normalize_number`, `normalize_expiry` (every spelling in section 4, and a year-first "2027-03"), `last_day`, `expired`, `expiry_of`, `part_value`, and `settle`. `frontend/app/view/vault/vault-card.ts` repeats the first four for the form as it is typed; both are held to the same table of numbers.
- **`settle` runs on every save by the person, an agent or an import** (`insert_unrecorded`, `update` after the parts are merged, and the import staging in `transfer.rs`). It writes the number as digits (a number with anything but digits, spaces and dashes is left as typed, because the Vault does not decide what a card is), the month as `01` to `12` and the year as four digits (a month or year that is not one refuses the save), reads the older `expiry` into them and drops it, works out `brand`, trims `zip`, and keeps `default` to `yes`. A card whose older `expiry` cannot be read keeps it as it was.
- **The expiry date is plain.** Section 7 said only the brand, the default flag and the last four are readable without the key. The list row, the expired chip and the reminders all need the expiry, and the Vault already keeps an `expires` date in the open for API keys and logins, so a card gets the same field: the last day of its expiry month, kept by `settle`. A card whose month and year are not saved yet has none (an entry saved before C1 gets one the first time it is saved; nothing is rewritten before). The number, code, name and ZIP stay sealed, and the expiry month and year are in the sealed parts too.
- **Reminders:** a card is reminded of the date like any entry (14 days, 3 days, on the day) and `Due.rotates` is false for it, so no rotation task is filed: the person replaces a card with the bank.
- **One default:** saving a card with `default: yes` takes the flag from the Vault's other cards (`keep_one_default`, which writes only the other cards' plain fields). It runs after the save has committed, so a failure to clear another card is logged, not turned into a failed save; C2's picker takes the most recently saved card when it finds two defaults.
- **A fill that asks for `expiry`** (a form field `card-expiry`, or a command's `<id>.expiry`) still gets it: `part_value` makes "MM/YY" from the month and year when a card has no older part. C2 replaces this with the shape rules of section 6.
- **Imports** still read a card's expiry as "MM/YYYY" and now also leave the staging step with `exp_month`, `exp_year`, `brand` and `expires`. A Vault export (kdbx, plain CSV) carries "Expiry month", "Expiry year" and "Billing ZIP" beside the older "Expiry"; an import reads them back.
- **The pane:** the card form has the grouped number with the brand's badge and the Luhn note, the month and year choices (a saved card shows its own, read from the plain date), the code's label and length by brand, the billing ZIP, and a default switch. The list row has the brand badge, the last four, a Default pill and an Expired chip. Not built in C1, and left for the phases that need them: the reveal of a card's new parts (they reveal like any part), and the picker (C2).
- **Restores and rotations** work the brand and `expires` out again from the value now in the entry (`resettle_card`), so the list and the reminders follow a restored card.
- **Clearing an expiry:** the form's Clear on the Expiry row removes the month, the year and the older single part, and with them the date and its reminders.
- **Not yet, and closed in C3:** `Browser capture` and `Terminal capture` write a card's sealed parts without `settle`; a captured replacement for the older `expiry` part of a card that already has a month and year would be undone by its next save. C3 makes capture write a card through `settle` (and a captured `expiry` into the month and year).

## As built (C2a)

C2 is two PRs: this one (the whole-card fill) and C2b (the picker and payment frames).

- **One call, one question.** `Browser` `fill` with `vault: "<card id>"` (no part) or `vault: "card"` (the default card) takes any card field of a form as its `ref`. `card_target.rs` resolves the card and reads its brand, last four and expiry for the approval; `server/browser_vault_fill/card.rs` asks the host for the form's card fields, asks the person once ("Visa •••• 4242, expires 03/27", into "the card fields: number, expiry, security code, name, ZIP"), opens the card only after the yes, and fills each field through the same `vault_fill` as a single one, which checks the origin and the field's markup again. `<id>.<part>` is unchanged.
- **The host's `card_form` route** (`vault_card_form.rs`) lists, in snapshot order, the fields of the same frame that fit a card slot, with what each takes (a choice, `maxlength`, placeholder, `pattern`). Password and one-time-code fields are never listed. A form that has none of number, expiry or security code is refused with the single-field form of the call (a ZIP or a name alone is a single-field fill).
- **Slots:** `card-exp-month`, `card-exp-year` and `card-zip` join the others, in the host's classifier (`cc-exp-month`, `cc-exp-year`, a billing `postal-code`, or the usual words; a shipping or delivery ZIP is not matched) and in `slots.rs`.
- **Shape** (`card_fill.rs`, pure): the one-field expiry is `MM/YY`, `MM/YYYY`, `MMYY` or `MMYYYY` by `placeholder`, `maxlength` and a `pattern` that asks for four digits; month and year text fields get `03` and `2027` (`27` for `maxlength` 2 or a `YY` placeholder); the number is grouped only where the field has room and shows spaces in its placeholder; a code field shorter than the code is left empty ("this field takes 3 digits and the card's code has 4"); a part the card lacks is listed by name as left.
- **`<select>` fill:** the host's `vault_fill` takes `choose: "month" | "year"` and picks the option whose value or label matches (`3`, `03`, `March`, `Mar`; `2027`, `27`), after the same origin and markup checks as a typed field, then fires `input` and `change`. No matching option chooses nothing and says so without the value.
- **Expired cards** ask every time: no "Always" is offered and a grant given earlier does not count. A current card with a grant for the site fills without asking. The grant is the same one a single-field fill uses (this card, this site).
- **The answer** names `parts_filled`, `parts_left` and `parts_failed` with a reason that never holds a value (an error that quotes a part is cleaned before it is returned), and the same "value: [secret: hidden]" as a single field. The record line names the parts filled.
- **Not yet, and closed in C2b:** the picker in the approval (until then, `card` with no default is refused with a message saying to name a card), the payment-frame pair in the approval (a field in a cross-origin frame is not listed by `card_form`, which looks in the frame of the given `ref`), and the key-event fallback with the hand-off message.
- **The list in a narrow pane:** the macOS and Linux look checks of C1 and C2a found that the six columns (about 530 px) do not fit the pane's default width (about 372 px), so the Value column and the row's buttons were out of view. Below 600 px of pane width a row now stacks (label and id across the top, then kind, details, value and date wrapping together, the buttons last) and the column headings are hidden; above it the table is unchanged. This applies to every section of the Vault list, not only cards. The brand badge wraps above a card's last four, which stay whole.

## As built (C2b, the picker)

C2b is two PRs: this one (the picker in the approval) and the payment frames.

- **The picker.** `Browser` `fill` with `vault: "card"` and no default card, or with a card's id when the person has more than one, asks with a "Card" choice: every card of the Vaults the agent may use, the default first and then the most recently saved, each as the label and "Visa •••• 4242, expires 03/27" (or "expired 03/2020"), the default marked. The agent's own card, or the default, is preselected; with none, nothing is and no yes is possible until one is chosen (Deny always is). A card the agent named that is granted for the site and current still fills without a question, as before.
- **The answer.** `UseRequest.choices` (`UseChoice`: id, label, detail, expired, default; no number, brand or last four beyond that line, which only the person's own windows receive) and `UseAnswer.choice`. The host's answer route accepts only an id from the question's own choices (or the preselected card when none is sent), and never "Always" for an expired card or a question that did not offer it. The card the person picks is one of those opened for the question, so a card changed while they decided is still refused when its parts are opened. A grant from "Always" is for the card picked on that site.
- **What the agent learns.** The same as before: `filled_from_vault` is the id of the card filled (the picked one), the parts filled and left by name; never the brand, last four or expiry.
- **`vault: "card"` with no card saved** is refused with a message saying so, and nobody is asked.
- **Not yet, and C2b's second PR:** a card field in a cross-origin frame, the frame's origin beside the page's site in the approval, and the key-event typing fallback with the hand-off message.
