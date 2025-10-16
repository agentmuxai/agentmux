# AgentMux WaveTerm-Inspired UI Redesign Specification

**Version:** 1.0
**Date:** 2025-10-15
**Status:** Draft

---

## Executive Summary

Transform AgentMux Desktop from a dashboard-centric UI to a **terminal-first, multi-pane workspace** inspired by WaveTerm. The redesign prioritizes direct terminal interaction with Claude agents while tucking management controls into minimal menus and modals.

**Core Philosophy:** Terminal work is primary, management is secondary.

---

## 1. Current UI Problems

### Pain Points
- **Dashboard-heavy**: Starts with bus controls/stats instead of terminal
- **Too many buttons**: Multiple tabs fighting for attention
- **Hidden terminal**: Users must navigate to access Claude CLI
- **No multi-agent support**: Can't work with multiple agents simultaneously
- **Management UI clutter**: Bus controls, agent lists consume screen space

### User Workflow Mismatch
Users want to:
1. **Open app → immediately interact with Claude**
2. **Work with multiple agents in split panes**
3. **Rarely manage bus/agents** (set-and-forget)

Current UI forces:
1. ~~Open app → see bus dashboard~~
2. ~~Click through tabs to find terminal~~
3. ~~Management UI always visible~~

---

## 2. Design Goals

### Primary Goals
1. **Terminal-first startup** - Claude CLI ready immediately
2. **Multi-pane workspace** - Side-by-side agent terminals
3. **Minimal chrome** - Hide management UI behind menus
4. **Zero navigation** - No tabs, direct access

### Inspiration: WaveTerm
- **Split panes** for multiple terminals
- **Clean, minimal UI** with dropdown menus
- **Terminal-centric** design language
- **Persistent sessions** across app restarts

---

## 3. New UI Architecture

### 3.1 Startup Experience

**On app launch:**
```
┌──────────────────────────────────────────────────┐
│ ☰ Menu   AgentMux Desktop              [_][□][X]│
├──────────────────────────────────────────────────┤
│                                                  │
│  ┌──────────────────────────────────────────┐   │
│  │                                          │   │
│  │   Claude Agent 1                         │   │
│  │                                          │   │
│  │   > Ready to help...                     │   │
│  │                                          │   │
│  │                                          │   │
│  └──────────────────────────────────────────┘   │
│                                                  │
└──────────────────────────────────────────────────┘
```

**Key Features:**
- **Single pane terminal** with Claude already spawned
- **Hamburger menu** (☰) for all controls
- **Clean title bar** with window controls only
- **No tabs**, **no buttons**, **no clutter**

---

### 3.2 Split Pane Layout

**User splits pane (via menu or keyboard shortcut):**
```
┌──────────────────────────────────────────────────┐
│ ☰ Menu   AgentMux Desktop              [_][□][X]│
├──────────────────────────────────────────────────┤
│                                                  │
│  ┌──────────────────┬───────────────────────┐   │
│  │ Claude Agent 1   │ Claude Agent 2        │   │
│  │                  │                       │   │
│  │ > Working on     │ > Ready to help...    │   │
│  │   pulse deploy   │                       │   │
│  │                  │                       │   │
│  └──────────────────┴───────────────────────┘   │
│                                                  │
└──────────────────────────────────────────────────┘
```

**Split Options:**
- Vertical split (side-by-side)
- Horizontal split (top-bottom)
- Resizable dividers
- Up to 4 panes (2×2 grid)

---

### 3.3 Hamburger Menu Structure

**☰ Menu** → Dropdown with all management options:

```
☰ AgentMux
├─ Agents
│  ├─ Spawn New Agent...
│  ├─ Agent List (modal)
│  └─ Kill All Agents
├─ Layout
│  ├─ Split Vertical
│  ├─ Split Horizontal
│  ├─ Close Current Pane
│  └─ Reset to Single Pane
├─ Bus
│  ├─ Start Bus
│  ├─ Stop Bus
│  └─ Bus Info (modal)
├─ Messages
│  ├─ Message Stream (modal)
│  └─ Clear Message History
├─ View
│  ├─ Show Debug Console
│  └─ Toggle Full Screen
├─ Settings
│  └─ Preferences...
└─ Help
   ├─ Documentation
   └─ About AgentMux
```

---

### 3.4 Modals for Management

**Agent List Modal** (opened from menu):
```
┌────────────────────────────────────────┐
│  Active Agents                    [X]  │
├────────────────────────────────────────┤
│                                        │
│  Agent1 (agent-12345)                  │
│    PID: 45678  |  Status: Running      │
│    [Kill] [Focus in Pane 1]            │
│                                        │
│  Agent2 (agent-67890)                  │
│    PID: 11223  |  Status: Running      │
│    [Kill] [Focus in Pane 2]            │
│                                        │
│  [Spawn New Agent]                     │
│                                        │
└────────────────────────────────────────┘
```

**Bus Info Modal**:
```
┌────────────────────────────────────────┐
│  Message Bus Status               [X]  │
├────────────────────────────────────────┤
│                                        │
│  Status:        ● Running              │
│  Endpoint:      ws://localhost:8765    │
│  Connected:     2 agents               │
│  Messages/sec:  0.5                    │
│  Total msgs:    127                    │
│                                        │
│  [Stop Bus]                            │
│                                        │
└────────────────────────────────────────┘
```

**Message Stream Modal**:
```
┌────────────────────────────────────────┐
│  Message Stream                   [X]  │
├────────────────────────────────────────┤
│                                        │
│  [Agent1 → Agent2] "Deploy complete"   │
│  [Agent2 → Agent1] "Logs checked"      │
│                                        │
│  [Pause] [Clear] [Export]              │
│                                        │
└────────────────────────────────────────┘
```

---

## 4. Implementation Plan

### Phase 1: Terminal-First Startup (Week 1)
- **Remove** Dashboard as default view
- **Auto-spawn** Claude agent on startup
- **Display** single terminal pane immediately
- **Hide** all management UI behind menu

**Acceptance Criteria:**
- App opens directly to Claude terminal
- No tabs visible
- Hamburger menu accessible
- Terminal responsive to keyboard input

---

### Phase 2: Hamburger Menu (Week 2)
- **Implement** menu component with dropdown
- **Migrate** all controls to menu items
- **Add** keyboard shortcuts for menu actions
- **Test** menu accessibility

**Menu Actions:**
- Bus controls (start/stop)
- Agent spawning/killing
- Layout management
- Modal triggers

---

### Phase 3: Split Pane Layout (Week 3)
- **Implement** pane splitter component
- **Add** resizable dividers
- **Support** vertical/horizontal splits
- **Enable** up to 4 panes (2×2 grid)
- **Keyboard shortcuts** for splits

**Key Interactions:**
- Drag divider to resize
- Click pane to focus
- Ctrl+W to close pane
- Ctrl+T to split vertical

---

### Phase 4: Modals for Management (Week 4)
- **Build** modal component system
- **Create** Agent List modal
- **Create** Bus Info modal
- **Create** Message Stream modal
- **Ensure** modals don't block terminal work

**Modal Features:**
- Click outside to close
- ESC to dismiss
- Overlay darkens background
- Non-blocking (can interact with terminals behind)

---

### Phase 5: Session Persistence (Week 5)
- **Save** pane layout on exit
- **Restore** pane configuration on startup
- **Remember** which agents were running
- **Auto-reconnect** to existing Claude processes

**Persistence Scope:**
- Pane count and split orientation
- Agent instances per pane
- Window size/position
- Menu preferences

---

## 5. Technical Architecture

### 5.1 Component Structure

```
App.tsx
├── Menubar.tsx (hamburger menu)
├── PaneContainer.tsx (split layout manager)
│   ├── Pane.tsx (individual terminal pane)
│   │   └── EmbeddedTerminal.tsx (xterm instance)
│   └── Divider.tsx (resizable split)
└── ModalManager.tsx
    ├── AgentListModal.tsx
    ├── BusInfoModal.tsx
    └── MessageStreamModal.tsx
```

### 5.2 State Management

```typescript
interface AppState {
  layout: {
    panes: PaneConfig[];
    orientation: 'horizontal' | 'vertical' | 'grid';
  };
  agents: Map<string, AgentInstance>;
  bus: {
    running: boolean;
    stats: BusStats;
  };
  modals: {
    activeModal: ModalType | null;
  };
}
```

### 5.3 Storage

**localStorage keys:**
- `agentmux.layout` - Pane configuration
- `agentmux.agents` - Last active agents
- `agentmux.preferences` - User settings

---

## 6. User Experience Flows

### 6.1 First-Time User
1. Opens AgentMux Desktop
2. Sees single Claude terminal, already running
3. Types question immediately
4. Discovers menu when needed

### 6.2 Power User
1. Opens app → split into 3 panes
2. Spawns agents in each pane
3. Works with multiple agents simultaneously
4. Rarely opens menus (uses keyboard shortcuts)

### 6.3 Troubleshooting
1. Opens hamburger menu
2. Selects "Bus Info"
3. Checks bus status in modal
4. Closes modal, continues work

---

## 7. Design Specifications

### 7.1 Colors
- **Background:** `#1E1E1E` (dark terminal)
- **Foreground:** `#D4D4D4` (light text)
- **Divider:** `#3E3E3E` (subtle)
- **Menu background:** `#252526`
- **Modal overlay:** `rgba(0,0,0,0.7)`

### 7.2 Typography
- **Monospace font:** `'JetBrains Mono', 'Fira Code', monospace`
- **UI font:** `'Inter', system-ui, sans-serif`
- **Sizes:** Terminal 14px, UI 13px

### 7.3 Spacing
- **Pane padding:** 8px
- **Menu item height:** 32px
- **Modal padding:** 24px
- **Divider width:** 4px

---

## 8. Success Metrics

### Quantitative
- **Startup to terminal:** < 1 second
- **Menu access time:** < 500ms
- **Split pane transition:** < 200ms
- **Modal open time:** < 150ms

### Qualitative
- Users report "immediate productivity"
- Zero complaints about "can't find terminal"
- Positive feedback on "clean UI"
- Users adopt split pane workflow

---

## 9. Migration Strategy

### Backward Compatibility
- **Keep** old Dashboard component (hidden)
- **Add** "Classic View" menu option
- **Preserve** existing Tauri commands
- **Maintain** bus/agent APIs

### Rollout Plan
1. **Beta release** with new UI (v0.4.0-beta)
2. **Collect feedback** from early adopters
3. **Iterate** on UX issues
4. **Stable release** with new UI as default (v0.4.0)
5. **Deprecate** classic view (v0.5.0)

---

## 10. Open Questions

1. **Keyboard shortcuts** - Which shortcuts to support?
   - Ctrl+T for split?
   - Ctrl+W for close pane?
   - Ctrl+Shift+P for menu?

2. **Pane indicators** - How to show which pane is active?
   - Border color?
   - Title bar highlight?
   - Glow effect?

3. **Agent labels** - How to label panes?
   - Auto-generate (Agent1, Agent2)?
   - User-defined names?
   - Show instance ID?

4. **Menu position** - Left or top-left corner?
   - Currently shows top-left (☰ Menu)
   - Could move to top-right
   - Could be floating button

---

## 11. Future Enhancements

### Post v0.4.0
- **Tabs within panes** (multiple agents per pane)
- **Drag-and-drop** pane rearrangement
- **Custom layouts** (save/load named layouts)
- **Collaborative sessions** (share terminals over network)
- **Terminal themes** (user-customizable colors)
- **Pane snapshots** (save/restore terminal state)

---

## Appendix A: WaveTerm Comparison

| Feature | WaveTerm | AgentMux (New) |
|---------|----------|----------------|
| Split panes | ✅ | ✅ |
| Minimal UI | ✅ | ✅ |
| Hamburger menu | ✅ | ✅ |
| Terminal-first | ✅ | ✅ |
| Tabs | ✅ | 🔜 v0.5.0 |
| Themes | ✅ | 🔜 v0.5.0 |
| Cloud sync | ✅ | ❌ Not planned |
| AI integration | ❌ | ✅ Claude CLI |

---

## Appendix B: Mockups

See companion file: `SPEC_WAVETERM_UI_REDESIGN_MOCKUPS.md`

---

**Next Steps:**
1. Review and approve this spec
2. Create mockups in Figma
3. Begin Phase 1 implementation
4. Schedule weekly design reviews
