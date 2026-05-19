---
type: patch
---

feat(launch-modal): integration test pinning the memory-change regression (Stage 2g, closes spec)

Adds Vitest + @solidjs/testing-library + jsdom integration test that
mounts AgentLaunchModalPanel with mocked RPCs, drives the user flow
programmatically, and asserts the auth panel doesn't reappear after a
Memory dropdown change. Pins the §6.10 acceptance criterion with a
fast (~360ms) jsdom-based test instead of standing up real
Playwright/WebDriver against CEF.

Approach + library choice rationale:
docs/specs/SPEC_LAUNCH_MODAL_INTEGRATION_TESTS_2026_05_19.md.

Closes Stage 2 of the launch-modal state-machine hardening spec.
