# PR #179 Cross-Platform Verification Report

**PR:** fix: restore CSS-native zoom calc for Windows
**Date:** 2026-03-19
**Author:** AgentA (Windows)

---

## Guarantee: This PR Does NOT Change macOS or Linux Behavior

### Proof by Code Path Analysis

The ONLY function that changed is `applyChromeZoomCSS` in `zoom.ts`. Here is a line-by-line comparison of what runs on each platform:

#### Linux

**Current main (working):**
```typescript
if (PLATFORM === PlatformLinux || factor <= 1) {
    headerWidth = "100vw";
}
document.documentElement.style.setProperty("--chrome-header-width", headerWidth);
// Result: --chrome-header-width = "100vw"
// CSS: var(--chrome-header-width, 100vw) → "100vw"
```

**This PR:**
```typescript
if (PLATFORM === PlatformLinux || factor <= 1) {
    document.documentElement.style.setProperty("--chrome-header-width", "100vw");
}
// Result: --chrome-header-width = "100vw"
// CSS: var(--chrome-header-width, calc(100vw / var(--zoomfactor, 1))) → "100vw"
```

**Identical outcome.** The CSS custom property `--chrome-header-width` is set to `"100vw"` in both cases. The CSS `var()` resolves to the same value. The fallback expression is never reached.

#### macOS

**Current main (working):**
```typescript
} else if (PLATFORM === PlatformMacOS) {
    headerWidth = "100%";
}
document.documentElement.style.setProperty("--chrome-header-width", headerWidth);
// Result: --chrome-header-width = "100%"
// CSS: var(--chrome-header-width, 100vw) → "100%"
```

**This PR:**
```typescript
} else if (PLATFORM === PlatformMacOS) {
    document.documentElement.style.setProperty("--chrome-header-width", "100%");
}
// Result: --chrome-header-width = "100%"
// CSS: var(--chrome-header-width, calc(100vw / var(--zoomfactor, 1))) → "100%"
```

**Identical outcome.** The CSS custom property `--chrome-header-width` is set to `"100%"` in both cases. The CSS `var()` resolves to the same value. The fallback expression is never reached.

#### Windows (the fix)

**Current main (broken):**
```typescript
} else {
    headerWidth = `calc(100vw / ${factor})`;
}
document.documentElement.style.setProperty("--chrome-header-width", headerWidth);
// Result: --chrome-header-width = "calc(100vw / 1.5)" (literal string)
// CSS: var(--chrome-header-width, 100vw) → "calc(100vw / 1.5)" (static, not zoom-aware)
```

**This PR:**
```typescript
} else {
    document.documentElement.style.removeProperty("--chrome-header-width");
}
// Result: --chrome-header-width is UNSET
// CSS: var(--chrome-header-width, calc(100vw / var(--zoomfactor, 1)))
//   → fallback: calc(100vw / var(--zoomfactor, 1))
//   → browser evaluates in zoom context (the original working behavior)
```

**Different outcome — intentionally.** This is the Windows fix.

---

### CSS File Change Analysis

**Current main:**
```scss
width: var(--chrome-header-width, 100vw);
```

**This PR:**
```scss
width: var(--chrome-header-width, calc(100vw / var(--zoomfactor, 1)));
```

**Impact on Linux/macOS:** NONE. When `--chrome-header-width` is set (which it is for both Linux and macOS), the fallback expression is never evaluated. The `var()` function uses the set value and ignores the fallback entirely. Per the CSS spec:

> "If the custom property named by the first argument to var() is animation-tainted, and the var() is being used in a property that is not animatable, treat the custom property as having its initial value for the purpose of performing the substitution."

More relevantly:
> "If the value of the custom property named by the first argument to var() is anything but the initial value, replace the var() function by the value of the corresponding custom property."

Since `--chrome-header-width` IS set to `"100vw"` (Linux) or `"100%"` (macOS) by the JS, the fallback `calc(100vw / var(--zoomfactor, 1))` is **never used** on those platforms.

---

### Summary

| Platform | `--chrome-header-width` set by JS? | Value | Fallback used? | Behavior change? |
|----------|-----------------------------------|-------|----------------|-----------------|
| Linux | YES → `"100vw"` | Same as main | No | **NO CHANGE** |
| macOS | YES → `"100%"` | Same as main | No | **NO CHANGE** |
| Windows | NO → removed | N/A | Yes → `calc(100vw / var(--zoomfactor, 1))` | **YES — this is the fix** |

---

### Edge Case: `PLATFORM` Default

`PLATFORM` defaults to `"darwin"` before `setPlatform()` runs. If `applyChromeZoomCSS` is called before platform init:

- `PLATFORM === PlatformLinux` → false (it's "darwin")
- `PLATFORM === PlatformMacOS` → true → sets `--chrome-header-width: 100%`

This means on Windows, if zoom init runs early, the macOS path executes. However:
- `initChromeZoom()` is called with `DEFAULT_ZOOM = 1.0`
- At `factor <= 1`, the Linux branch fires first: `--chrome-header-width = "100vw"`
- The macOS branch is never reached for the init call
- User-triggered zoom (factor > 1) only happens after `setPlatform()` has run

**No edge case risk.**
