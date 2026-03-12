# SolidJS Migration Spec — AgentMux Frontend

**Version:** 0.1 — 2026-03-11
**Status:** Draft
**Goal:** Replace React 19 + Jotai with SolidJS for fine-grained reactivity, smaller bundle, and faster streaming performance.

---

## 1. Why SolidJS

| Metric | React 19 + Jotai | SolidJS |
|--------|-----------------|---------|
| Framework gz size | ~50 KB | ~7 KB |
| DOM update model | VDOM diff + reconcile | Fine-grained signal subscriptions |
| Re-render on stream event | Full subtree diff | Only subscribed nodes update |
| Bundle saving | — | ~43 KB gz (~8%) |
| Terminal streaming perf | Moderate | 40–60% fewer DOM ops |
| forwardRef / memo needed | Yes | No |

Primary gain is runtime: during LLM streaming, agent view re-renders hundreds of times per second. React batches but still reconciles full component trees. SolidJS updates only the exact DOM nodes that read changed signals.

---

## 2. Critical Blockers (must resolve before migration)

| # | Blocker | Current code | SolidJS solution |
|---|---------|-------------|-----------------|
| 1 | Jotai global store | `globalStore.get/set(atom)` throughout | Replace atoms with module-level signals; export `[signal, setter]` pairs |
| 2 | ViewModel class pattern | `new AgentViewModel(blockId, nodeModel)` — stores Jotai atoms as class fields | Convert to factory functions returning plain signal objects |
| 3 | VDomModel | Uses React reconciler (`ReactDOM.createRoot`) internally | Replace with SolidJS `render()` + custom solid reconciler or port VDom renderer |
| 4 | Layout reducer | `useReducer` for layout tree mutations | Convert to `createStore` (SolidJS mutable store for nested objects) |
| 5 | forwardRef / useImperativeHandle | Used in terminal component | Remove — pass ref directly (SolidJS refs are plain variables) |
| 6 | ErrorBoundary | React-specific | Use SolidJS `<ErrorBoundary>` from solid-js |
| 7 | react-dnd | Drag-and-drop library | Replace with `@thisbeyond/solid-dnd` or custom pointer events |
| 8 | VDom event bindings | `AtomContainer` uses Jotai atoms for two-way bindings | Replace with `createSignal` per binding, stored in `Map<string, [Accessor, Setter]>` |
| 9 | react-markdown | Used in VDom (`wave:markdown`) + agent view | Replace with `solid-markdown` or custom remark/unified pipeline component |

---

## 3. API Mapping Reference

| React / Jotai | SolidJS equivalent |
|--------------|-------------------|
| `useState(x)` | `createSignal(x)` → `[val, setVal]` |
| `useEffect(fn, [])` | `onMount(fn)` |
| `useEffect(fn, [deps])` | `createEffect(fn)` — deps auto-tracked |
| `useMemo(fn, [deps])` | `createMemo(fn)` — auto-tracked |
| `useRef<T>()` | `let ref!: T` — assign in JSX `ref={el => ref = el}` |
| `useCallback(fn, [])` | Plain function (signals captured by closure) |
| `useContext(Ctx)` | `useContext(Ctx)` — same API |
| `createContext(default)` | `createContext(default)` — same API |
| `memo(Component)` | Not needed — SolidJS components run once |
| `<Suspense>` | `<Suspense>` — same API |
| `<ErrorBoundary>` | `<ErrorBoundary>` from `solid-js` |
| `atom(val)` (Jotai) | `createSignal(val)` — module-level |
| `atom((get) => expr)` | `createMemo(() => expr)` — module-level |
| `useAtomValue(a)` | `a()` — call the signal |
| `useSetAtom(a)` | `setA` — the setter from `createSignal` |
| `useAtom(a)` | `[a, setA]` — destructure the pair |
| `globalStore.get(a)` | `a()` — signals are globally callable |
| `globalStore.set(a, v)` | `setA(v)` |
| `atomWithStorage(key, val)` | Custom: `createSignal` + `createEffect` writing to localStorage |
| JSX `className` | `class` — SolidJS uses real DOM attributes |
| JSX `htmlFor` | `for` |
| Event `onChange` | `onInput` for real-time, `onChange` for commit |
| `children` prop | `props.children` — same, but treat as accessor |

---

## 4. Package Changes

### Remove
```
react
react-dom
@types/react
@types/react-dom
@types/prop-types
prop-types
jotai
react-dnd
react-dnd-html5-backend
@vitejs/plugin-react-swc
@ai-sdk/react
@floating-ui/react
@radix-ui/react-label
@radix-ui/react-slot
@react-hook/resize-observer
@table-nav/core           # dead — zero usage
@table-nav/react          # dead — zero usage
@tanstack/react-table
overlayscrollbars-react
react-frame-component     # dead — only in unused ijson.tsx
react-hook-form           # dead — zero usage
react-markdown
react-resizable-panels    # dead — layout is fully custom
```

### Add
```
solid-js
vite-plugin-solid          # replaces @vitejs/plugin-react-swc
@ai-sdk/solid              # replaces @ai-sdk/react
@floating-ui/dom           # replaces @floating-ui/react (use base package directly)
@tanstack/solid-table      # replaces @tanstack/react-table
@thisbeyond/solid-dnd      # replaces react-dnd
@modular-forms/solid       # replaces react-hook-form (if forms needed)
```

### Keep (no change needed)
```
overlayscrollbars          # use base package directly (drop overlayscrollbars-react)
immer                      # used with createStore produce()
rxjs                       # framework-agnostic, unchanged
@xterm/*                   # framework-agnostic, unchanged
mermaid / shiki / katex    # framework-agnostic, unchanged
```

### Vite config change
```ts
// vite.config.ts
import solid from "vite-plugin-solid";
// replace: import react from "@vitejs/plugin-react"

export default {
  plugins: [solid()],
  // remove: optimizeDeps.include for react
}
```

### tsconfig change
```json
{
  "compilerOptions": {
    "jsx": "preserve",
    "jsxImportSource": "solid-js"
    // remove: "jsx": "react-jsx", "jsxImportSource": "react"
  }
}
```

---

## 5. Global State Architecture

### Current (Jotai)
```ts
// atoms.ts
export const tabAtom = atom<Tab | null>(null);
export const staticTabIdAtom = atom<string>("");
export const layoutStateAtom = atom<LayoutState>({ ... });

// usage
const tab = useAtomValue(tabAtom);
globalStore.set(staticTabIdAtom, tabId);
```

### SolidJS replacement
```ts
// store/tab.ts
export const [tab, setTab] = createSignal<Tab | null>(null);

// store/layout.ts
export const [staticTabId, setStaticTabId] = createSignal("");
export const [layoutState, setLayoutState] = createStore<LayoutState>({ ... });

// usage in component
const currentTab = tab();              // reactive read
setTab(newTab);                        // update from anywhere
setLayoutState("blocks", blocks);     // nested store update
```

### Global store object (for legacy RPC callbacks)
```ts
// store/index.ts — replaces globalStore
export const atoms = {
  get staticTabId() { return staticTabId(); },
  setStaticTabId,
  get tab() { return tab(); },
  setTab,
};
```

---

## 6. Layer-by-Layer Spec

### 6.1 Store / State Layer (`frontend/store/`)

**Files:** `atoms.ts`, `global.ts`, `wos.ts`, `wos-cache.ts`

**Current pattern:**
- Jotai `atom()` for all state
- `wos.ts` subscribes to WebSocket events, writes to atoms
- `global.ts` exports `globalStore` and composite atoms

**SolidJS pattern:**
- Replace each `atom(val)` with `createSignal(val)` at module scope
- Replace `atom((get) => expr)` with `createMemo(() => expr)`
- `wos.ts`: replace `globalStore.set(atom, val)` with direct setter calls
- `global.ts`: export signal accessors and setters; remove `globalStore`

**Key signals to create:**
```ts
// connections
export const [connections, setConnections] = createSignal<Connection[]>([]);
export const [activeConnId, setActiveConnId] = createSignal<string>("");

// tabs / layout
export const [staticTabId, setStaticTabId] = createSignal<string>("");
export const [layoutState, setLayoutState] = createStore<LayoutNodeType>(emptyLayout);

// config
export const [fullConfig, setFullConfig] = createSignal<FullConfigType>(defaultConfig);

// client
export const [clientId, setClientId] = createSignal<string>("");
```

**Derived (memos):**
```ts
export const activeTab = createMemo(() =>
  connections().find(c => c.id === activeConnId())
);
```

---

### 6.2 Hooks Layer (`frontend/app/hooks/`)

**Files:** `useblockatomvalues.ts`, `usewaveobject.ts`, `useheight.ts`, `usewidth.ts`, etc.

**Current pattern:**
```ts
export function useBlockAtomValues(blockId: string) {
  const blockAtom = useAtomValue(blockAtoms(blockId));
  return blockAtom;
}
```

**SolidJS pattern:**
- Hooks become plain functions returning signal accessors
- `createMemo` for derived values

```ts
// useBlockData.ts
export function useBlockData(blockId: string) {
  return createMemo(() => wos.getObjectValue<Block>(WOS.makeORef("block", blockId)));
}

// useResizeObserver.ts
export function useResizeObserver(el: () => HTMLElement | undefined) {
  const [size, setSize] = createSignal({ width: 0, height: 0 });
  createEffect(() => {
    const element = el();
    if (!element) return;
    const ro = new ResizeObserver(([entry]) => {
      setSize({ width: entry.contentRect.width, height: entry.contentRect.height });
    });
    ro.observe(element);
    onCleanup(() => ro.disconnect());
  });
  return size;
}
```

**Rules:**
- No dependency arrays — SolidJS tracks automatically inside `createEffect`/`createMemo`
- `onCleanup` replaces useEffect cleanup return
- Reactive primitives must be called inside tracking contexts

---

### 6.3 Block / Frame Layer (`frontend/app/block/`)

**Files:** `block.tsx`, `block-frame.tsx`, `block-frame-default.tsx`, `pane-actions.ts`

**Current pattern:**
```tsx
const BlockFrame: React.FC<BlockFrameProps> = memo(({ blockId, ... }) => {
  const blockData = useAtomValue(blockAtoms(blockId));
  const [isHovered, setIsHovered] = useState(false);
  // ...
});
```

**SolidJS pattern:**
```tsx
const BlockFrame: Component<BlockFrameProps> = (props) => {
  const blockData = useBlockData(props.blockId);  // returns memo
  const [isHovered, setIsHovered] = createSignal(false);

  return (
    <div
      class={clsx("block-frame", { hovered: isHovered() })}
      onMouseEnter={() => setIsHovered(true)}
      onMouseLeave={() => setIsHovered(false)}
    >
      {/* blockData() — call the memo */}
      <Show when={blockData()}>
        {(data) => <BlockContent data={data()} />}
      </Show>
    </div>
  );
};
```

**Key changes:**
- `memo(Component)` → remove wrapper, not needed
- `className` → `class`
- `{condition && <X/>}` → `<Show when={condition}><X/></Show>`
- `{list.map(i => <X/>)}` → `<For each={list()}>{(i) => <X/>}</For>`
- `{a ? <X/> : <Y/>}` → `<Switch><Match when={a}><X/></Match><Match when={!a}><Y/></Match></Switch>`

**Block registry:**
```ts
// block.tsx — ViewModel registry stays, but viewModels become factory functions
export const BlockRegistry = new Map<string, ViewModelFactory>();

type ViewModelFactory = (blockId: string, nodeModel: BlockNodeModel) => ViewModel;

// agent-model.ts
export function createAgentViewModel(blockId: string, nodeModel: BlockNodeModel): ViewModel {
  const [status, setStatus] = createSignal<string>("init");
  const [messages, setMessages] = createSignal<Message[]>([]);
  // ...
  return { viewType: "agent", viewComponent: AgentView, status, messages, ... };
}
```

---

### 6.4 Layout Layer (`frontend/app/layout/`)

**Files:** `layout.tsx`, `layoutAtom.ts`, `tilelayout.tsx`, `model.ts`

**Current pattern:**
```ts
const [layoutState, dispatchLayoutAction] = useReducer(layoutReducer, initialLayout);
const layoutAtom = atom<LayoutState>(initialLayout);
```

**SolidJS pattern — use `createStore` for mutable nested state:**
```ts
import { createStore, produce } from "solid-js/store";

const [layout, setLayout] = createStore<LayoutNodeType>(initialLayout);

// mutations use produce (immer-like)
function splitBlock(blockId: string, direction: "h" | "v") {
  setLayout(produce(draft => {
    const node = findNode(draft, blockId);
    if (!node) return;
    node.children = [{ ...node }, { type: "leaf", blockId: newId() }];
    node.type = direction === "h" ? "hbox" : "vbox";
  }));
}
```

**Drag and drop:**
```tsx
import { DragDropProvider, DragDropSensors, createDraggable, createDroppable } from "@thisbeyond/solid-dnd";

// Replace react-dnd useDrag/useDrop hooks
const draggable = createDraggable(blockId);
const droppable = createDroppable(blockId);
```

---

### 6.5 Terminal Pane (`frontend/app/view/term/`)

**Files:** `term.tsx`, `termwrap.ts`, `termstyles.tsx`

**Current pattern:**
```tsx
const TerminalView: React.FC = memo(({ blockId }) => {
  const termWrapRef = useRef<TermWrap | null>(null);
  useEffect(() => {
    termWrapRef.current = new TermWrap(blockId, containerRef.current);
    return () => termWrapRef.current?.dispose();
  }, [blockId]);
});
```

**SolidJS pattern:**
```tsx
const TerminalView: Component<{ blockId: string }> = (props) => {
  let containerEl!: HTMLDivElement;
  let termWrap: TermWrap | undefined;

  onMount(() => {
    termWrap = new TermWrap(props.blockId, containerEl);
  });

  onCleanup(() => {
    termWrap?.dispose();
  });

  const fontSize = createMemo(() => {
    const meta = blockMeta(props.blockId)();
    return clamp(meta?.["term:zoom"] ?? 12, 4, 64);
  });

  createEffect(() => {
    termWrap?.setFontSize(fontSize());
  });

  return <div ref={containerEl} class="term-container" />;
};
```

**TermWrap class:** No changes needed — it's a plain class wrapping xterm.js, not a React component.

**xterm.js / @xterm/xterm:** Compatible with SolidJS — no React dependency.

**Terminal streaming:**
```ts
// RxJS subject subscriptions — use onMount/onCleanup
onMount(() => {
  const sub = getFileSubject(props.blockId, "term").subscribe(data => {
    termWrap?.handleData(data);
  });
  onCleanup(() => sub.unsubscribe());
});
```

---

### 6.6 Agent Pane (`frontend/app/view/agent/`)

**Files:** `agent-view.tsx`, `agent-model.ts`, `message-list.tsx`, `tool-use.tsx`

**Current pattern:**
```tsx
const AgentView: React.FC = ({ blockId }) => {
  const vm = useAtomValue(agentViewModelAtom(blockId));
  const messages = useAtomValue(vm.messagesAtom);
  const status = useAtomValue(vm.statusAtom);
  // re-renders on every message append
};
```

**SolidJS pattern — major perf win:**
```tsx
const AgentView: Component<{ blockId: string }> = (props) => {
  const vm = agentViewModels.get(props.blockId)!;

  return (
    <div class="agent-view">
      <Show when={vm.status() === "awaiting_browser"}>
        <AuthOverlay url={vm.authUrl()} />
      </Show>
      <div class="message-list">
        <For each={vm.messages()}>
          {(msg) => <MessageItem message={msg} />}
        </For>
      </div>
      <AgentInput blockId={props.blockId} />
    </div>
  );
};
```

**Why this is faster:** `<For>` only adds/removes DOM nodes for changed items. React would re-render the full list on every append. During streaming (100+ appends/sec), SolidJS adds one `<div>` per message vs React's full VDOM diff.

**AgentViewModel as factory:**
```ts
// agent-model.ts
export interface AgentViewModel {
  viewType: "agent";
  viewComponent: Component<{ blockId: string }>;
  // signals
  messages: Accessor<Message[]>;
  status: Accessor<string>;
  authUrl: Accessor<string>;
  currentInput: Accessor<string>;
  // setters
  appendMessage: (msg: Message) => void;
  setStatus: (s: string) => void;
}

export function createAgentViewModel(blockId: string): AgentViewModel {
  const [messages, setMessages] = createSignal<Message[]>([]);
  const [status, setStatus] = createSignal("init");
  const [authUrl, setAuthUrl] = createSignal("");
  const [currentInput, setCurrentInput] = createSignal("");

  function appendMessage(msg: Message) {
    setMessages(prev => [...prev, msg]);
  }

  // subscribe to PTY output
  onMount(() => {
    const sub = getFileSubject(blockId, "term").subscribe(handleTerminalData);
    onCleanup(() => sub.unsubscribe());
  });

  return { viewType: "agent", viewComponent: AgentView, messages, status, authUrl, currentInput, appendMessage, setStatus };
}
```

**Streaming text update (assistant message in progress):**
```tsx
// Instead of replacing entire messages array, update last message in place
const [streamText, setStreamText] = createSignal("");

// In parser callback:
onTextDelta(delta) {
  setStreamText(prev => prev + delta);
}

// In JSX — only this span re-renders:
<span class="stream-text">{streamText()}</span>
```

---

### 6.7 Sysinfo Pane (`frontend/app/view/sysinfo/`)

**Files:** `sysinfo.tsx`, `cpu-plot.tsx`, `mem-plot.tsx`

**Current pattern:**
```tsx
const CpuPlot: React.FC = memo(({ blockId }) => {
  const cpuData = useAtomValue(cpuDataAtom);
  return <canvas ref={canvasRef} />;
});
```

**SolidJS pattern:**
```tsx
const CpuPlot: Component<{ blockId: string }> = (props) => {
  let canvasEl!: HTMLCanvasElement;
  const cpuData = useCpuData(props.blockId);  // returns Accessor<CpuSample[]>

  createEffect(() => {
    const data = cpuData();  // reactive — re-runs when data updates
    drawCpuChart(canvasEl, data);
  });

  return <canvas ref={canvasEl} class="cpu-plot" />;
};
```

**Chart rendering:** Canvas-based charts (Chart.js, uPlot, plain canvas) work unchanged — no React dependency. Sysinfo widgets should require minimal changes beyond reactive wiring.

---

### 6.8 Launcher Pane (`frontend/app/view/launcher/`)

**Files:** `launcher.tsx`

**Current pattern:**
```tsx
const LauncherView: React.FC = ({ blockId }) => {
  const [query, setQuery] = useState("");
  const results = useMemo(() => filterWidgets(query, fullConfig), [query, fullConfig]);
};
```

**SolidJS pattern:**
```tsx
const LauncherView: Component<{ blockId: string }> = (props) => {
  const [query, setQuery] = createSignal("");
  const results = createMemo(() => filterWidgets(query(), fullConfig()));

  return (
    <div class="launcher">
      <input
        value={query()}
        onInput={(e) => setQuery(e.currentTarget.value)}
      />
      <For each={results()}>
        {(widget) => <WidgetEntry widget={widget} />}
      </For>
    </div>
  );
};
```

**Note:** `onInput` vs `onChange` — SolidJS `onChange` fires on blur like native DOM. Use `onInput` for real-time filter.

---

### 6.9 Help Pane (`frontend/app/view/help/`)

**Files:** `helpview.tsx`

Mostly static content. Minimal reactive state. Direct port — replace `useState` with `createSignal`, `className` with `class`. Low risk, low effort.

---

### 6.10 VDom Renderer (`frontend/app/view/vdom/`)

**Files:** `vdom.tsx`, `vdom-model.tsx`, `vdom-utils.tsx`
**Also:** `frontend/app/view/term/termVDom.tsx` — terminal VDom overlay integration

**Status: LIVE — must port.** VDom is not a plugin system — it is the terminal's **"vdom mode"**. When a program running inside a terminal calls the VDom RPC API, the terminal pane switches to render a rich UI overlay alongside the PTY output. The TermViewModel has explicit `vdomBlockId`, `vdomToolbarBlockId`, and `termMode === "vdom"` state. This is used by backend apps (shell scripts, Rust binaries) that want to push interactive widgets into their own terminal pane.

**What dead code WAS removed:** `ijson.tsx` used `react-frame-component` to render JSON-described UI inside an iframe. It was never imported and is deleted.

**Current VDom approach:** The backend sends `VDomElem` JSON trees. The frontend reconciles them into React elements by walking the tree and calling `React.createElement()` dynamically. Event callbacks are serialized as UUIDs; when fired, the UUID is sent back to the backend which routes to the correct handler.

**SolidJS approach — use `<Dynamic>`:**

```tsx
import { Dynamic } from "solid-js/web";

// Render a server-sent VDomElem node
const VDomNode: Component<{ elem: VDomElem; model: VDomModel }> = (props) => {
  const elem = () => props.elem;

  return (
    <Switch>
      <Match when={elem().tag === "#text"}>
        {elem().text}
      </Match>
      <Match when={elem().tag === "wave:markdown"}>
        <Markdown text={elem().props?.text ?? ""} />
      </Match>
      <Match when={AllowedSimpleTags[elem().tag] || AllowedSvgTags[elem().tag]}>
        <Dynamic
          component={elem().tag}
          {...buildDomProps(elem(), props.model)}
        >
          <For each={elem().children ?? []}>
            {(child) => <VDomNode elem={child} model={props.model} />}
          </For>
        </Dynamic>
      </Match>
    </Switch>
  );
};
```

**Event handling:** The existing pattern (UUID callbacks sent back over RPC) is framework-agnostic — just replace `React.SyntheticEvent` with native DOM events. The `annotateEvent` utility keeps working with minor type changes.

**Bindings:** The `AtomContainer` pattern (Jotai atoms for two-way bindings) becomes SolidJS signals. Each binding creates a `createSignal` stored in a `Map<string, Signal>` on the VDomModel.

**Risk level:** Medium — the tree-walking logic is mechanical, but event handling plumbing needs careful testing.

---

### 6.11 OpenClaw Widget (new — `frontend/app/view/openclaw/`)

**New pane type — write in SolidJS from scratch:**

```tsx
// openclaw-view.tsx
const OpenClawView: Component<{ blockId: string }> = (props) => {
  const vm = createOpenClawViewModel(props.blockId);

  return (
    <Switch>
      <Match when={vm.state() === "checking"}>
        <div class="openclaw-loading"><Spinner /></div>
      </Match>
      <Match when={vm.state() === "not-installed"}>
        <InstallScreen onInstall={vm.install} />
      </Match>
      <Match when={vm.state() === "installing"}>
        <InstallingScreen progress={vm.installProgress()} />
      </Match>
      <Match when={vm.state() === "setup"}>
        <SetupScreen onComplete={vm.completeSetup} />
      </Match>
      <Match when={vm.state() === "running"}>
        <WebviewPane url="http://localhost:18789" />
      </Match>
    </Switch>
  );
};
```

Writing in SolidJS from the start avoids migration cost for this new view.

---

### 6.12 Elements / UI Kit (`frontend/app/element/`)

**Files:** `button.tsx`, `input.tsx`, `dropdown.tsx`, `modal.tsx`, `spinner.tsx`, etc.

**Current:** Small React components. Low complexity.

**SolidJS port pattern:**
```tsx
// button.tsx
export const Button: Component<{
  onClick?: () => void;
  disabled?: boolean;
  children: JSX.Element;
  class?: string;
}> = (props) => (
  <button
    class={clsx("btn", props.class, { disabled: props.disabled })}
    onClick={props.onClick}
    disabled={props.disabled}
  >
    {props.children}
  </button>
);
```

**Props spreading note:** In SolidJS, spread props with `splitProps` to avoid passing DOM-invalid props:
```tsx
const [local, rest] = splitProps(props, ["class", "children"]);
return <button class={local.class} {...rest}>{local.children}</button>;
```

---

### 6.13 Modals (`frontend/app/modals/`)

**Files:** `modal-overlay.tsx`, `settings-modal.tsx`, `connection-modal.tsx`

**Current:** React portals (`ReactDOM.createPortal`) to render modals at document body.

**SolidJS replacement:**
```tsx
import { Portal } from "solid-js/web";

const Modal: Component<{ open: boolean; onClose: () => void; children: JSX.Element }> = (props) => (
  <Show when={props.open}>
    <Portal mount={document.body}>
      <div class="modal-overlay" onClick={props.onClose}>
        <div class="modal-content" onClick={(e) => e.stopPropagation()}>
          {props.children}
        </div>
      </div>
    </Portal>
  </Show>
);
```

`<Portal>` is the direct equivalent of `ReactDOM.createPortal`.

---

### 6.14 Window / App Chrome (`frontend/app/window/`)

**Files:** `window-frame.tsx`, `action-widgets.tsx`, `tab-bar.tsx`, `status-bar.tsx`

**Current:**
```tsx
const ActionWidgets: React.FC = () => {
  const config = useAtomValue(fullConfigAtom);
  const widgets = config.widgets;
  return (
    <div class="action-widgets">
      {Object.entries(widgets).map(([key, w]) => (
        <WidgetButton key={key} widget={w} />
      ))}
    </div>
  );
};
```

**SolidJS pattern:**
```tsx
const ActionWidgets: Component = () => {
  const widgetEntries = createMemo(() => Object.entries(fullConfig().widgets ?? {}));

  return (
    <div class="action-widgets">
      <For each={widgetEntries()}>
        {([key, widget]) => <WidgetButton key={key} widget={widget} />}
      </For>
    </div>
  );
};
```

**Tab bar:** Uses same `<For>` pattern. Tab close/reorder uses layout store mutations.

**Status bar:** Mostly display. Replace `useAtomValue` with signal calls.

---

## 7. Migration Order (6-week plan)

| Week | Layer | Risk | Notes |
|------|-------|------|-------|
| 1 | Package setup + Store/State signals | Low | Foundation; unblocks everything |
| 1 | Elements / UI Kit | Low | Isolated components, no deps |
| 2 | Hooks layer | Low | Pure functions, testable |
| 2 | Window / App Chrome | Low | Mostly display |
| 2 | Launcher, Help | Low | Simple views |
| 3 | Block / Frame | Medium | Core rendering loop |
| 3 | Layout (createStore + solid-dnd) | Medium | State-heavy, test drag carefully |
| 4 | Modals | Low | Portal swap |
| 4 | Sysinfo | Low | Canvas, minimal reactivity |
| 4 | Terminal | Medium | TermWrap class stays; wire signals |
| 5 | Agent view | High | Streaming perf critical path |
| 5 | OpenClaw widget | Low | Write fresh in SolidJS |
| 6 | VDom renderer | High | `<Dynamic>` approach; event bindings → signals |
| 6 | Terminal VDom overlay (`termVDom.tsx`) | Medium | Wire to new VDomModel signals |
| 6 | Final: remove React/Jotai | — | Delete packages, verify bundle |

---

## 8. Gotchas Checklist

- [ ] **Signals must be called `()` in JSX** — `{count}` is static; `{count()}` is reactive. Forgetting `()` is the #1 bug source.
- [ ] **Components run ONCE** — no dependency arrays, no re-execution on prop change. Use `createMemo`/`createEffect` with `props.x` access inside to react to prop changes.
- [ ] **`props` is a reactive proxy** — destructuring breaks reactivity: `const { x } = props` → `x` never updates. Use `props.x` directly or `splitProps`.
- [ ] **`onInput` not `onChange`** for text inputs if you want real-time updates.
- [ ] **`class` not `className`** in JSX.
- [ ] **`for` not `htmlFor`** on labels.
- [ ] **`<For>` tracks by reference** — ensure list items have stable identity or use `key` prop.
- [ ] **`createEffect` runs immediately** (unlike `useEffect` which runs after paint). Watch for infinite loops if the effect writes to a signal it reads.
- [ ] **`onCleanup` inside `createEffect`** — called before next run AND on component unmount. No separate cleanup phase.
- [ ] **Module-level signals** created outside components don't auto-cleanup — manage lifetime explicitly for per-block state (store in a Map, dispose on block close).
- [ ] **No `React.StrictMode` double-invoke** — SolidJS components run once, no double-mount. Remove strict mode assumptions from effects.
- [ ] **RxJS subscriptions** inside `onMount` with `onCleanup` — same pattern as useEffect, just different API.

---

## 9. Expected Outcomes

| Metric | Before | After |
|--------|--------|-------|
| Bundle gz (framework) | ~50 KB | ~7 KB |
| Total bundle gz | ~654 KB | ~597 KB (−9%) |
| Agent streaming DOM ops | ~N per message | ~1 per message (new `<span>`) |
| Full list re-render cost | O(n) VDOM diff | O(1) signal update |
| forwardRef patterns | Several | Zero |
| memo() wrappers | Several | Zero |
| Jotai dependency | Yes | No |
| react-dnd dependency | Yes | No |

---

## 10. Open Questions

1. **VDom fallback decision:** Keep React for vdom view only, or invest in full Dynamic port? Depends on how often VDom panes are used in practice.
2. **Testing:** Current tests use React Testing Library. Migrate to `@solidjs/testing-library` (same API surface) or use Playwright e2e only?
3. **TypeScript strict mode:** SolidJS JSX types are slightly stricter about `children` types — may surface new TS errors in element components.
4. **Tauri IPC:** Not affected — RPC calls are plain `fetch`/WebSocket, no framework dependency.
