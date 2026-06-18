---
type: patch
---

fix(agent): auto-resume after AskUserQuestion on persistent (Claude) panes. When a turn ends while a question is still parked, the CLI abandons the pending tool_use and the answer's control_response is silently dropped — the agent stalls ("dead air"). `answer_question` now arms a short post-answer check: if no stdout activity follows the control_response, the answer is re-delivered as a directive follow-up user message so the turn resumes. Gated on output activity, so it is mutually exclusive with a real resume and never double-delivers. Mirrors the one-shot controllers' existing message fallback (SPEC_ASK_USER_QUESTION §9/§10.1).
