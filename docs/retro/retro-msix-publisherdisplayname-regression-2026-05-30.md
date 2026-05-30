# RETRO — MSIX `PublisherDisplayName` regression (Store ingest rejection)

**Date:** 2026-05-30
**Author:** AgentX
**Severity:** Medium — no data lost, no bad artifact shipped to users, but it
blocked the Microsoft Store submission and is a **third occurrence of the same
one-line bug** (fixed twice in March, regressed in May).
**Area:** MSIX packaging — `packaging/msix/AppxManifest.xml.template`,
`scripts/package-msix.ps1`, `docs/specs/SPEC_MSIX_PACKAGING_2026_05_30.md`.

---

## Summary

Partner Center rejected `AgentMux_0.40.1_x64.msix` on upload:

> Package acceptance validation error: The `PublisherDisplayName` element in the
> app manifest of `AgentMux_0.40.1_x64.msix` is **AgentMux Corp**, which doesn't
> match your publisher display name: **AgentMux**.

The manifest's `<PublisherDisplayName>` must equal the Partner Center account's
**publisher display name** (`AgentMux`) *verbatim*. It was set to the **legal
entity** name (`AgentMux Corp.`) — the value that is correct for copyright
headers (`LEGAL.md`, `LICENSE`, source headers) but wrong for this Store field.

This exact bug was already found and fixed **twice in March**. The fix lived
only inside `src-tauri/AppxManifest.xml`; when that directory was deleted, the
corrected value was lost, and the manifest was later recreated from scratch with
the wrong value again.

## Who changed it / timeline

| Date | Commit | Author | `PublisherDisplayName` |
|------|--------|--------|------------------------|
| 2026-03-17 | `dfd1174b` | agentx-workflow[bot] | added `src-tauri/AppxManifest.xml` = **AgentMux Corp** (wrong from day one) |
| 2026-03-18 | `058c242e` | AgentY (#158) | merged, still **AgentMux Corp** |
| **2026-03-21** | `bb391461` | agentx-workflow[bot] | **"Fix MSIX PublisherDisplayName to match Store account": → AgentMux** ✅ (1st fix) |
| 2026-03-26 | `e5389810` → `f74eb732` | AgentX (#240) | "correct MSIX identity for Partner Center" — reaffirmed **→ AgentMux** ✅ |
| 2026-04-03 | `12333fa2` | AgentA | "delete src-tauri and all Tauri build infrastructure" — deleted the manifest that held the correct **AgentMux**; the fix died with the file |
| **2026-05-30** | `cc5f7368` | **AgentX** | rebuilt packaging for the CEF app — created a **new** `packaging/msix/AppxManifest.xml.template` and re-typed **AgentMux Corp**, regressing the March fixes. Also baked the wrong value into the spec with a "✅ recovered" annotation. |

**The regression was introduced by AgentX in `cc5f7368`** (the May 30 CEF
packaging PR, #1209). It was not a malicious or external change — it was a
reconstruction-from-scratch that reached for the legal-entity name and trusted
the Store catalog API's `DeveloperName` field ("AgentMux Corp") over the actual
correction history.

## Root cause

Two compounding causes:

1. **Field conflation.** `PublisherDisplayName` (Store account display name,
   ingest-validated = `AgentMux`) looks interchangeable with the legal entity
   `AgentMux Corp.` and with the catalog API's `DeveloperName` (`AgentMux Corp`).
   They are not. Only the Partner Center *publisher display name* is valid here.

2. **The fix was never captured durably.** The Mar 21 / #240 corrections lived
   *only* as a literal value inside `src-tauri/AppxManifest.xml`. There was no
   guard, no test, and no prominent doc explaining *why* it had to be `AgentMux`.
   When the file was deleted (Apr 3) and the manifest recreated (May 30), there
   was nothing to copy from and nothing to stop the regression. A fix that lives
   only as a value in a file that later gets deleted is not a fix — it's a
   countdown.

## Resolution

- `AppxManifest.xml.template` → `<PublisherDisplayName>AgentMux</PublisherDisplayName>`, with an inline comment explaining the field and pointing here.
- **Build-time guard** in `package-msix.ps1`: `$EXPECTED_PUBLISHER_DISPLAY_NAME = "AgentMux"`; the render step now asserts the manifest value matches and throws before `makeappx` runs (mirrors the existing Publisher-hash guard). A wrong value now fails the *build*, not the Store upload.
- Spec doc corrected: the §10 recovery table no longer claims `DeveloperName` = the publisher display name, and explicitly warns they differ.
- Rebuilt the MSIX; the guard passes; re-uploaded to Partner Center.

## Lessons / prevention

- **A fix isn't done until it can't regress.** The durable artifact is the
  guard + the comment + this retro, not the corrected value in one file.
- **Don't trust a catalog/API field name at face value** for a value that has a
  precise, separately-validated meaning. `DeveloperName` ≠ `PublisherDisplayName`.
- **When recreating tooling that replaces deleted code, mine the git history of
  the thing you're replacing** (`git log -G` on the old file) for fixes that
  were applied to it — the deletion doesn't carry the lessons forward.
- The two values to keep straight, permanently:
  - **Legal entity** (copyright, LICENSE, NOTICE, source headers): `AgentMux Corp.`
  - **Store publisher display name** (`PublisherDisplayName`): `AgentMux`
