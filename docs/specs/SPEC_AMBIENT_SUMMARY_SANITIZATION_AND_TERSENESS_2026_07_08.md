# SPEC: Ambient Haiku-Summary Sanitization + Terseness Pass

**Date:** 2026-07-08
**Status:** Draft — root cause confirmed, fix proposed, not yet implemented
**Related:** `agentmux-srv/src/server/app_api/session.rs`, `agentmux-srv/src/ambient/mod.rs`,
`frontend/app/view/agent/hooks/useNextPromptSuggestion.ts`,
`frontend/app/view/agent/components/AgentFooter.tsx`,
`docs/specs/SPEC_AMBIENT_GHOST_TEXT_NEXT_PROMPT_2026_07_03.md`,
`docs/specs/SPEC_AMBIENT_MODEL_CALLS_FRAMEWORK_2026_07_03.md`

---

## 0. TL;DR

Users occasionally see a literal ` ``` ` / newline / ` ``` ` blob appear in the
**composer's empty input box** — "for no reason." Root cause confirmed: it's
the ghost-text next-prompt suggestion (`term:next_prompt_suggestion`), which
is Haiku-generated text written verbatim into the textarea's `placeholder`
attribute with zero sanitization anywhere in the pipeline. Haiku occasionally
wraps its one-line answer in a markdown code fence (a common model habit,
worse when recent activity is code-heavy); an empty or near-empty fence
renders as exactly the reported artifact.

Two changes:

1. **Sanitize** ambient-call output at its one choke point
   (`invoke_ambient_haiku_call`) so a fence/quote-wrapped response can never
   reach a UI surface unmodified — fixes the bug for this and every future
   ambient-call purpose, not just this one.
2. **Tighten** the three existing ambient prompt templates to explicitly
   forbid markdown/formatting and push harder for terse, factual, flourish-free
   phrasing — reduces how often the model reaches for a fence in the first
   place, and addresses the separate ask that these summaries read as overly
   wordy/flowery today.

---

## 1. Root cause (confirmed by reading current `main`, not inferred)

Three RPCs share one Haiku call site,
`invoke_ambient_haiku_call()` (`agentmux-srv/src/server/app_api/session.rs:522`):

| Purpose | Prompt (current) | Consumer | Rendering surface |
|---|---|---|---|
| `activity_summary` | "Summarize in {word_target} words or fewer what is currently being worked on. Use a short terse phrase with no quotes or punctuation." (`session.rs:180-184`) | pane header / Swarm label | text node |
| `subagent_name` | "Give a concise ~5-word name for this task. No punctuation, no quotes, no preamble — respond with just the name." (`session.rs:262-266`) | Swarm row label | text node |
| `next_prompt_suggestion` | "...predict ONE short, natural next message... Respond with just that message and nothing else — no quotes, no explanation, no preamble. If nothing plausible comes to mind, respond with an empty string." (`session.rs:435-441`) | **composer ghost text** | `<textarea placeholder>` |

None of the three prompts mention markdown or code fences. All three trust
the model to follow "no quotes/no punctuation/no preamble" instructions
literally, with no server-side check.

The confirmed leak path, end to end:

1. `session.rs:444` — `invoke_ambient_haiku_call` returns Haiku's raw text,
   unmodified, as `suggestion`.
2. `useNextPromptSuggestion.ts:100-106` — writes `result.suggestion` straight
   to `term:next_prompt_suggestion` block meta if non-empty; no trim, no
   format check.
3. `AgentFooter.tsx:346-349` — `placeholder` memo: `if (suggestion) return
   suggestion;` — returned verbatim, no processing.
4. `AgentFooter.tsx:734` — `placeholder={placeholder()}` on the composer
   `<textarea>`. A native HTML placeholder renders literal text, including
   backticks and embedded newlines, exactly as given — no markdown
   interpretation exists anywhere in this path (unlike message-history
   rendering, which goes through the markdown renderer).

So when Haiku answers with, e.g., a fenced empty block instead of a bare
sentence or an actual empty string, the textarea shows the fence literally:
line 1 "` ``` `", line 2 "` ``` `" — the exact artifact reported ("```
newline ```" in the input box, no visible cause because it's ghost text, not
anything the user typed).

`activity_summary` and `subagent_name` share the same unsanitized call site
and are exposed to the same failure mode; they're just less visible (short
header labels rarely get long enough content for a fence to look "empty" the
same way) and haven't been reported, but the same fix protects them.

Note: an older full-CLI, multi-sentence "session digest" banner
(`SessionDigestCommand` / `useSessionDigest.ts` / `SessionDigestBanner.tsx`)
existed at one point but is confirmed **removed** from current `main` — the
three purposes above are the entire current surface for AI-generated summary
text.

---

## 2. Proposed fix

### 2.1 Sanitize at the shared choke point (primary fix)

Add a sanitizer inside `invoke_ambient_haiku_call` (`session.rs`), applied to
every purpose uniformly, immediately before the text is returned:

```rust
/// Defends against the model wrapping its answer in markdown despite being
/// told not to — every ambient-call prompt asks for a bare line of text, but
/// instruction-following isn't guaranteed. Strips a single wrapping code
/// fence (with or without a language tag), strips wrapping single/double/
/// smart quotes, then trims whitespace. Never partial — either the whole
/// wrapper comes off or the text is returned as-is (a fence-like substring
/// in the middle of real content is left alone; only wrapping is stripped).
fn sanitize_ambient_text(raw: &str) -> String {
    let mut s = raw.trim();
    if let Some(inner) = strip_wrapping_fence(s) {
        s = inner.trim();
    }
    s.trim_matches(|c| matches!(c, '"' | '\'' | '\u{201C}' | '\u{201D}' | '\u{2018}' | '\u{2019}'))
        .trim()
        .to_string()
}
```

(`strip_wrapping_fence` — regex or manual scan for a leading ` ``` `
optionally followed by a language tag and newline, and a trailing ` ``` `,
returning the content between; `None` if the string isn't fully wrapped.)

Call sites (`register_session_activity_summary`, `generate_subagent_name`,
`register_session_next_prompt_suggestion`) need no change — they all funnel
through `invoke_ambient_haiku_call`, so fixing it there fixes all three at
once, and any future ambient-call purpose gets the same protection for free
(same "one place owns it" principle the ghost-text spec already used for
frontend precedence).

After sanitizing, treat an empty result exactly like today's "empty string"
/ "CLI failed" path:
- `next_prompt_suggestion`: `empty_suggestion_result()` — no ghost text shown.
- `activity_summary`: already filtered by `.filter(|(summary, _)| !summary.is_empty())` (`session.rs:188`) — falls through unchanged.
- `subagent_name`: already checked via `if name.is_empty()` (`session.rs:273`) — falls through unchanged.

No caller-side changes needed beyond this one function.

### 2.2 Tighten the three prompts (defense in depth + the terseness ask)

Reduce how often the model reaches for formatting at all, and make the
output read as flatter/more factual — same intent, tighter wording:

- **`activity_summary`** (`session.rs:180-184` / `341-344`, both copies —
  keep them identical, they're duplicated for the pushed-vs-pulled call
  paths):
  `"Summarize in {word_target} words or fewer what is currently being worked
  on. Plain text only — no markdown, no code fences, no backticks, no
  quotes, no punctuation, no preamble."`
- **`subagent_name`** (`session.rs:262-266`):
  `"Give a concise ~5-word name for this task. Plain text only — no
  markdown, no code fences, no backticks, no punctuation, no quotes, no
  preamble. Respond with just the name."`
- **`next_prompt_suggestion`** (`session.rs:435-441`):
  `"Based on this recent activity, predict ONE short, natural next message
  the user might send to continue the conversation. Respond with just that
  message and nothing else — plain text only, no markdown, no code fences,
  no backticks, no quotes, no explanation, no preamble. If nothing plausible
  comes to mind, respond with an empty string."`

These are prompt-level nudges, not a substitute for §2.1 — a model can still
ignore instructions; §2.1 is what makes the fix actually reliable.

### 2.3 Not doing (out of scope)

- No frontend sanitization layer. The composer already treats
  `term:next_prompt_suggestion` as trusted, pre-cleaned text once §2.1 ships
  server-side; adding a second stripping layer client-side would be the kind
  of redundant defensive code the project avoids, and would invite drift
  between the two copies.
- No settings toggle / behavior change to when these calls fire — this spec
  only touches output cleanliness and phrasing, not the Ambient Model Call
  gateway's admission/coalescing logic, which is unrelated and already works.

---

## 3. Test plan

- Unit tests for `sanitize_ambient_text` (colocated with
  `invoke_ambient_haiku_call` in `session.rs`):
  - Plain sentence passes through unchanged.
  - ` ```\ntext\n``` ` → `text`.
  - ` ```rust\ntext\n``` ` (language tag) → `text`.
  - ` ```\n``` ` (empty fence — the reported bug) → `""`.
  - Leading/trailing quote wrapping (`"text"`, `'text'`, curly quotes) → `text`.
  - A fence-like substring embedded mid-sentence (not wrapping the whole
    string) is left alone.
- `next_prompt_suggestion` regression case: feed a canned Haiku response of
  literal ` ```\n``` ` through the full handler, assert
  `NextPromptSuggestionResult.suggestion == ""` (so the frontend never writes
  it to block meta).
- Manual: force a code-heavy recent-activity window (e.g. a session that just
  ran a big diff/tool dump) and confirm the ghost-text suggestion that
  appears in the composer is a plain sentence, never a fence.

---

## 4. Files touched

| File | Change |
|---|---|
| `agentmux-srv/src/server/app_api/session.rs` | Add `sanitize_ambient_text` + `strip_wrapping_fence`; apply inside `invoke_ambient_haiku_call` before returning; tighten the three prompt strings (§2.2); unit tests. |

No frontend changes (§2.3).
