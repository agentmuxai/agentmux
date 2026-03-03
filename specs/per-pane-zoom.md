# Per-Pane Terminal Zoom Specification - AgentMux

**Version:** 1.0
**Status:** Draft
**Target Release:** 0.28.0
**Created:** 2026-02-15
**Author:** AgentX

---

## Executive Summary

This spec proposes **per-pane zoom for terminal blocks**, allowing each terminal to have an independent zoom level while maintaining global app zoom for UI chrome. This follows the proven pattern already implemented for web blocks (`web:zoom`) and leverages AgentMux's existing per-block metadata system.

**Key Insight:** Per-pane terminal zoom is **architecturally straightforward** because:
- Terminal font sizing already uses per-block metadata (`term:fontsize`)
- Web blocks already implement per-pane zoom (`web:zoom`)
- Block metadata system supports atomic numeric values
- Settings menu pattern is established and proven

**Effort Estimate:** **Medium complexity** - 1-2 days
- Similar to existing web zoom implementation
- Requires UI menu additions
- Optional state preservation adds complexity

---

## Problem Statement

### Current Limitations

1. **Global zoom only**: Ctrl+/- zooms the entire app including UI chrome
2. **No terminal-specific zoom**: Cannot zoom individual terminals independently
3. **Font size workaround**: Users must manually change font size per terminal
4. **Inconsistent UX**: Web blocks have per-pane zoom, terminals don't

### User Impact

**Use Cases:**
- **Code review**: Zoom in on one terminal (logs) while keeping another normal (editor output)
- **Presentation**: Increase zoom on demo terminal without affecting entire UI
- **Accessibility**: Users with vision needs want per-terminal control
- **Multi-monitor**: Different zoom levels for different screen distances

**User Frustration:**
- Changing global zoom affects tabbar, widgets, all terminals simultaneously
- Font size menu is cumbersome (requires multiple clicks, limited sizes)
- No quick keyboard shortcut for terminal-specific zoom

---

## Requirements

### Functional Requirements

#### 1. **Per-Pane Zoom Controls**

**Method 1: Context Menu (Primary)**
- Right-click terminal → "Zoom" submenu → Select zoom percentage
- Range: 50%, 75%, 100%, 125%, 150%, 175%, 200%
- Default: 100% (null in metadata = use base font size)
- Include "Reset to Default" option (clears `term:zoom` metadata)

**Method 2: Keyboard Shortcuts (Optional)**
- Focused terminal only: Ctrl+Shift+= (zoom in), Ctrl+Shift+- (zoom out)
- Step size: 25% increments
- Visual indicator shows current zoom for 1.5s

#### 2. **Zoom Range**

| Property | Value | Rationale |
|----------|-------|-----------|
| Minimum | 50% (0.5x) | Maintain readability, prevent text too small |
| Maximum | 200% (2.0x) | Prevent extreme sizes, maintain layout |
| Default | 100% (1.0x) | Match base font size |
| Step size | 25% | Menu: discrete steps; Keyboard: consistent with global zoom |

**Why smaller range than web zoom (0.1-5x)?**
- Terminals use fixed-width fonts (readability critical)
- Extreme zoom breaks terminal layout grid
- Users have font size for large adjustments

#### 3. **Persistence**

- Store in block metadata: `term:zoom` (number, nullable)
- Persisted via `RpcApi.SetMetaCommand()`
- Null value = use default (100%)
- Survives app restart, tab switches, workspace changes

#### 4. **Visual Feedback**

**Transient Indicator (Optional):**
- Show "Terminal Zoom: 125%" for 1.5s when changed
- Position: top-right of terminal pane
- Fade in/out animation (same as global zoom indicator)
- Different color/style to distinguish from global zoom

**Status Bar (Optional, future):**
- Show zoom percentage in terminal footer/header
- Only display if zoom != 100%

#### 5. **Interaction with Existing Features**

**Global App Zoom:**
- Independent: Per-pane zoom multiplies base font size
- Example: Base = 12px, global zoom = 125% → 15px, then per-pane 150% → 22.5px
- Result: `fontSize = baseFontSize * globalZoom * paneZoom`

**Font Size Setting:**
- Per-pane zoom multiplies the terminal's configured font size
- User can have font size = 14px + zoom = 150% → effective 21px
- Font size menu and zoom menu are independent

**Connection-Level Font Size:**
- Connection font size overrides global setting (existing behavior)
- Per-pane zoom multiplies connection font size too

---

### Non-Functional Requirements

1. **Performance**: Zoom change applies instantly (<100ms)
2. **Smoothness**: No visible layout thrashing or flicker
3. **State Preservation**: Preserve scroll position when zoom changes
4. **Accessibility**: Zoom respects system font scaling settings
5. **Consistency**: UI/UX matches web block zoom pattern

---

## Technical Design

### Architecture Overview

```
┌─────────────────────────────────────────────┐
│        Terminal Context Menu                │
│     (term.tsx getSettingsMenuItems)         │
│                                             │
│  User selects: Zoom → 150%                 │
└─────────────────┬───────────────────────────┘
                  │
                  ▼
┌─────────────────────────────────────────────┐
│      Block Metadata Update                  │
│     (RpcApi.SetMetaCommand)                 │
│                                             │
│  SET term:zoom = 1.5                        │
└─────────────────┬───────────────────────────┘
                  │
                  ▼
┌─────────────────────────────────────────────┐
│       Jotai Atom Triggers                   │
│    (getBlockMetaKeyAtom change)             │
│                                             │
│  termZoomAtom detects new value             │
└─────────────────┬───────────────────────────┘
                  │
                  ▼
┌─────────────────────────────────────────────┐
│      Font Size Calculation                  │
│   (termFontSizeAtom recomputes)             │
│                                             │
│  effectiveSize = baseSize * zoomFactor      │
└─────────────────┬───────────────────────────┘
                  │
                  ▼
┌─────────────────────────────────────────────┐
│    Terminal Reconstruction                  │
│       (term.tsx useEffect)                  │
│                                             │
│  TermWrap created with new fontSize         │
│  Scroll position restored                   │
└─────────────────────────────────────────────┘
```

### Implementation Approach: **Font Size Multiplication**

**Chosen Approach:** Multiply `fontSize` by `term:zoom` factor

**Rationale:**
- ✅ Uses xterm.js native font size support
- ✅ Clean, deterministic rendering
- ✅ Consistent with existing font size architecture
- ✅ No CSS transform hacks or scaling artifacts
- ⚠️ Requires terminal reconstruction (acceptable tradeoff)

**Alternative (Rejected): CSS Transform Scaling**
- Uses `transform: scale(zoomFactor)` on terminal DOM
- ❌ Potential issues with text selection
- ❌ Interaction with WebGL renderer
- ❌ Pixel alignment artifacts
- ✅ Lighter-weight (no terminal recreation)
- **Decision:** Avoid for v1, consider for future optimization

---

### Code Changes

#### **1. Add Metadata Key to Types** (`frontend/types/gotypes.d.ts`)

```typescript
interface MetaType {
    // ... existing keys
    "term:zoom"?: number;  // Terminal zoom factor (0.5-2.0), null = 100%
}
```

#### **2. Create Zoom Atom** (`frontend/app/view/term/term.tsx`)

```typescript
// Add after termFontSizeAtom (around line 273)
this.termZoomAtom = useBlockAtom(blockId, "termzoomatom", () => {
    return jotai.atom<number>((get) => {
        const blockData = get(this.blockAtom);
        const zoomFactor = blockData?.meta?.["term:zoom"];

        // Validate range
        if (zoomFactor == null) {
            return 1.0;  // Default 100%
        }
        if (typeof zoomFactor !== "number" || isNaN(zoomFactor)) {
            return 1.0;
        }
        // Clamp to safe range
        return Math.max(0.5, Math.min(2.0, zoomFactor));
    });
});
```

#### **3. Modify Font Size Calculation** (`frontend/app/view/term/term.tsx`)

```typescript
// Update termFontSizeAtom to include zoom factor (around line 259-273)
this.fontSizeAtom = useBlockAtom(blockId, "fontsizeatom", () => {
    return jotai.atom<number>((get) => {
        const blockData = get(this.blockAtom);
        const fsSettingsAtom = getSettingsKeyAtom("term:fontsize");
        const settingsFontSize = get(fsSettingsAtom);
        const connName = blockData?.meta?.connection;
        const fullConfig = get(atoms.fullConfigAtom);
        const connFontSize = fullConfig?.connections?.[connName]?.["term:fontsize"];

        // Get base font size (existing logic)
        const baseFontSize = blockData?.meta?.["term:fontsize"] ?? connFontSize ?? settingsFontSize ?? 12;

        // Validate base font size
        if (typeof baseFontSize !== "number" || isNaN(baseFontSize) || baseFontSize < 4 || baseFontSize > 64) {
            return 12;
        }

        // Apply zoom factor (NEW)
        const zoomFactor = get(this.termZoomAtom);
        const effectiveFontSize = baseFontSize * zoomFactor;

        // Final validation
        return Math.max(4, Math.min(64, Math.round(effectiveFontSize)));
    });
});
```

#### **4. Add Zoom Menu** (`frontend/app/view/term/term.tsx`)

```typescript
// Add to getSettingsMenuItems() around line 543
getSettingsMenuItems(): ContextMenuItem[] {
    const blockData = globalStore.get(this.blockAtom);
    const fullMenu: ContextMenuItem[] = [];

    // ... existing menu items (font size, theme, etc.)

    // NEW: Terminal Zoom submenu
    const currentZoom = blockData?.meta?.["term:zoom"] ?? 1.0;
    const zoomLevels = [0.5, 0.75, 1.0, 1.25, 1.5, 1.75, 2.0];

    const zoomSubMenu: ContextMenuItem[] = zoomLevels.map((zoom: number) => {
        const percentage = Math.round(zoom * 100);
        return {
            label: `${percentage}%`,
            type: "checkbox",
            checked: Math.abs(currentZoom - zoom) < 0.01,  // Float comparison tolerance
            click: () => {
                RpcApi.SetMetaCommand(TabRpcClient, {
                    oref: WOS.makeORef("block", this.blockId),
                    meta: { "term:zoom": zoom === 1.0 ? null : zoom },  // null = default
                });
            },
        };
    });

    // Add separator and reset option
    zoomSubMenu.push({ type: "separator" });
    zoomSubMenu.push({
        label: "Reset to Default",
        click: () => {
            RpcApi.SetMetaCommand(TabRpcClient, {
                oref: WOS.makeORef("block", this.blockId),
                meta: { "term:zoom": null },
            });
        },
    });

    fullMenu.push({
        label: "Terminal Zoom",
        submenu: zoomSubMenu,
    });

    return fullMenu;
}
```

#### **5. Preserve Scroll Position** (Optional Enhancement)

```typescript
// In term.tsx useEffect for terminal initialization (around line 1015-1056)
useEffect(() => {
    // ... existing terminal creation code

    const prevScrollY = termRef.current?.terminal?.buffer?.active?.viewportY ?? 0;

    const termWrap = new TermWrap(
        blockId,
        connectElemRef.current,
        {
            // ... options including fontSize: termFontSize
        },
        waveOptions
    );

    // ... terminal setup

    // Restore scroll position after recreation
    if (prevScrollY > 0) {
        setTimeout(() => {
            termWrap.terminal?.scrollToLine(prevScrollY);
        }, 50);
    }

}, [termFontSize, /* other deps */]);
```

---

### Data Model

#### **Block Metadata Schema**

```typescript
{
    "term:zoom": number | null,  // Zoom factor (0.5-2.0), null = 100%
}
```

**Storage Location:** Backend block object → `meta` field → `term:zoom`

**Persistence:** Saved to database via `RpcApi.SetMetaCommand()`

**Fallback Behavior:**
- `null` or missing → 100% zoom (use base font size)
- Invalid value → clamp to 0.5-2.0 range
- NaN → fallback to 100%

#### **Effective Font Size Calculation**

```
effectiveFontSize = baseFontSize * termZoomFactor

Where:
  baseFontSize = block meta → connection → global setting → 12px
  termZoomFactor = term:zoom metadata (0.5-2.0)

Example:
  baseFontSize = 14px
  termZoomFactor = 1.5
  effectiveFontSize = 14 * 1.5 = 21px
```

---

### User Interface

#### **Context Menu Integration**

```
Right-click terminal
├─ Font Size
│  ├─ 10px
│  ├─ 12px ✓
│  └─ 14px
├─ Terminal Theme
│  └─ ...
├─ Terminal Zoom          ← NEW
│  ├─ 50%
│  ├─ 75%
│  ├─ 100% ✓
│  ├─ 125%
│  ├─ 150%
│  ├─ 175%
│  ├─ 200%
│  ├─ ─────────
│  └─ Reset to Default
├─ Transparency
└─ ...
```

**Menu Position:** After "Font Size", before "Terminal Theme" (logical grouping with visual settings)

#### **Visual Indicator (Optional)**

```
┌────────────────────────────────────────┐
│  Terminal Output                  125% │ ← Indicator (1.5s fade)
│                                        │
│  $ npm run build                       │
│  Building...                           │
│                                        │
└────────────────────────────────────────┘
```

**Styling:**
- Position: absolute, top-right corner
- Background: semi-transparent (rgba(0,0,0,0.8))
- Font: 11px, monospace
- Fade: 200ms in, hold 1.5s, 300ms out
- Z-index: above terminal, below modals

---

## Implementation Phases

### Phase 1: Core Functionality (MVP)
**Effort:** 1 day
**Deliverables:**
- [ ] Add `term:zoom` to MetaType interface
- [ ] Create `termZoomAtom` in term.tsx
- [ ] Modify `termFontSizeAtom` to multiply by zoom factor
- [ ] Add zoom submenu to context menu
- [ ] Test: Set zoom via menu, verify font size changes
- [ ] Test: Zoom persists across app restarts

**Success Criteria:**
- User can set per-terminal zoom via context menu
- Zoom persists in block metadata
- Terminal renders with correct font size
- No regressions to existing font size behavior

### Phase 2: Polish (Optional)
**Effort:** 0.5 days
**Deliverables:**
- [ ] Add transient zoom indicator (fade in/out)
- [ ] Preserve scroll position during zoom changes
- [ ] Add keyboard shortcuts (Ctrl+Shift+/-, Ctrl+Shift+0)
- [ ] Add status bar indicator (if zoom != 100%)

**Success Criteria:**
- Smooth UX with visual feedback
- No scroll position loss on zoom
- Keyboard shortcuts work for focused terminal

### Phase 3: Advanced (Future)
**Effort:** TBD
**Ideas:**
- Global + per-pane zoom interaction modes (additive vs multiplicative)
- Zoom synchronization across terminals in same workspace
- Preset zoom profiles (code review mode, presentation mode)
- Mouse wheel zoom (Ctrl+Alt+Wheel for focused terminal)

---

## Testing Plan

### Unit Tests

**test: termZoomAtom validation**
```typescript
test("termZoomAtom clamps zoom to 0.5-2.0 range", () => {
    // Set term:zoom = 5.0 (invalid)
    // Expect: termZoomAtom returns 2.0
});

test("termZoomAtom defaults to 1.0 when null", () => {
    // Set term:zoom = null
    // Expect: termZoomAtom returns 1.0
});
```

**test: font size calculation**
```typescript
test("effective font size = base * zoom", () => {
    // Base font size = 12px, zoom = 1.5
    // Expect: effectiveFontSize = 18px
});

test("font size clamped to 4-64px range", () => {
    // Base = 4px, zoom = 0.5 → effectiveFontSize = 2px
    // Expect: clamped to 4px minimum
});
```

### Integration Tests

**test: zoom menu interaction**
1. Right-click terminal
2. Click "Terminal Zoom" → "150%"
3. Verify: Block metadata updated (`term:zoom: 1.5`)
4. Verify: Terminal font size increased
5. Verify: Menu shows checkmark at 150%

**test: zoom persistence**
1. Set terminal zoom to 125%
2. Switch to different tab
3. Switch back to terminal tab
4. Verify: Zoom still 125%
5. Restart app
6. Verify: Zoom persists

**test: reset to default**
1. Set zoom to 200%
2. Click "Reset to Default"
3. Verify: `term:zoom` metadata = null
4. Verify: Font size returns to base size

### Manual Testing

**Scenarios:**
1. **Multi-terminal workflow**: Create 3 terminals, set different zooms (75%, 100%, 150%), verify independence
2. **Font size interaction**: Set font size = 14px, zoom = 150%, verify effective size = 21px
3. **Global zoom interaction**: Set global zoom = 125%, per-pane zoom = 150%, verify both apply
4. **Connection font size**: Connect to SSH with connection-level font size = 16px, set zoom = 75%, verify effective = 12px
5. **Edge cases**: Set extreme zooms (0.5, 2.0), verify layout remains usable

**Performance:**
- Measure time from menu click to terminal update (<100ms target)
- Verify no memory leaks with rapid zoom changes
- Test with large scrollback buffer (10,000 lines) + zoom change

---

## Risks & Mitigations

| Risk | Impact | Likelihood | Mitigation |
|------|--------|------------|------------|
| Terminal recreation slow | UX lag | Medium | Optimize TermWrap init, measure perf |
| Scroll position lost | User frustration | High | Implement scroll restoration |
| Metadata corruption | Zoom reset | Low | Validate + clamp in atom, handle null |
| Interaction with global zoom | Confusion | Medium | Clear documentation, visual indicators |
| Extreme zoom breaks layout | Unusable terminal | Low | Strict range limits (0.5-2.0) |

---

## Success Metrics

**Adoption:**
- 20% of users use per-terminal zoom within 1 month
- Average zoom level != 100% for 30% of terminals

**Performance:**
- Zoom change latency < 100ms (p95)
- No regressions in terminal rendering FPS

**Quality:**
- Zero bugs related to zoom metadata corruption
- Zero scroll position loss complaints

**User Satisfaction:**
- Feature mentioned positively in feedback/reviews
- No user complaints about UX confusion

---

## Future Enhancements

### 1. **Smart Zoom Presets**
- "Code Review Mode": Zoom all terminals to 125%
- "Presentation Mode": Zoom to 150% + increase transparency
- "Compact Mode": Zoom to 75% for dashboard/monitoring

### 2. **Zoom Synchronization**
- Option: "Synchronize zoom across workspace"
- Useful for multi-terminal workflows (all logs, all builds)

### 3. **Mouse Wheel Zoom**
- Ctrl+Alt+Wheel: Zoom focused terminal
- Same as browser devtools pattern

### 4. **Zoom History**
- Undo/redo zoom changes
- Useful when accidentally clicking wrong zoom level

### 5. **CSS Transform Optimization**
- Investigate CSS scaling as performance alternative
- Requires extensive testing for text selection, WebGL

---

## Open Questions

1. **Should per-pane zoom affect terminal header/footer?**
   - Current: Zoom affects only terminal content (xterm canvas)
   - Alternative: Zoom entire terminal block including chrome
   - **Decision:** Content only (matches web block behavior)

2. **Should zoom affect line spacing/cell height?**
   - Current: Font size change auto-adjusts line height
   - Alternative: Independent line height control
   - **Decision:** Auto-adjust (simpler UX)

3. **Should global zoom affect per-pane zoom calculation?**
   - Option A: Independent (`effectiveSize = base * paneZoom`)
   - Option B: Multiplicative (`effectiveSize = base * globalZoom * paneZoom`)
   - **Decision:** Option B (mirrors web block behavior, more intuitive)

4. **Should we add keyboard shortcuts in Phase 1?**
   - **Decision:** Phase 2 (menu-driven first, shortcuts are polish)

---

## Appendix: Code Locations Reference

| Component | File | Lines | Description |
|-----------|------|-------|-------------|
| Font size atom | `term.tsx` | 259-273 | Current font size calculation |
| Terminal init | `term.tsx` | 1015-1056 | TermWrap creation with fontSize |
| Settings menu | `term.tsx` | 543-643 | Context menu builder |
| Web zoom reference | `webview.tsx` | 569-622 | setZoomFactor pattern |
| Block metadata types | `gotypes.d.ts` | 582-679 | MetaType interface |
| Theme updater | `termtheme.ts` | 17-28 | Dynamic option updates |
| FitAddon | `fitaddon.ts` | 38-99 | Cell dimension calculations |

---

## Revision History

| Version | Date | Author | Changes |
|---------|------|--------|---------|
| 1.0 | 2026-02-15 | AgentX | Initial spec based on research |

---

**Next Steps:**
1. Review spec with team/stakeholders
2. Prioritize Phase 1 vs Phase 2 features
3. Spike: Test scroll position restoration approach
4. Implementation: Start with Phase 1 MVP

---

**References:**
- `SPEC_ZOOM.md` - Global app zoom specification
- Web block zoom implementation (`webview.tsx`)
- Terminal architecture research (Explore agent findings)
- xterm.js documentation: https://xtermjs.org/docs/api/terminal/interfaces/iterminaloptions/
