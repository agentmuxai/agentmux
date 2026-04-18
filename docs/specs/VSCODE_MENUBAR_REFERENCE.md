# VS Code Menu Bar — Reference for AgentMux

Source: `microsoft/vscode@main`, 2026-04-17. All quotes verbatim.

---

## 1. High-level architecture

```
+-------------------------------------------------------------------------+
| MenuRegistry  (src/vs/platform/actions/common/actions.ts)               |
|   static "File", "Edit", ... submenus, declared at startup              |
+--------------------+------------------------------+---------------------+
                     |                              |
            custom path (Win/Linux/Web)      native path (macOS, or      |
                     |                       "window.titleBarStyle":     |
                     v                       "native" on Win/Linux)      |
+---------------------------------+      +--------------------------------+
| CustomMenubarControl             |     | MenubarMainService (main proc) |
|   workbench/.../menubarControl.ts|     |  platform/menubar/electron-    |
|   – reads MenuRegistry           |     |  main/menubar.ts               |
|   – turns actions into IActions  |     |  – builds electron.Menu        |
|   – owns DOM container           |     |  – Menu.setApplicationMenu()   |
+----------------+-----------------+     +---------------+----------------+
                 |                                       |
                 v                                       v
+---------------------------------+      +--------------------------------+
| MenuBar widget                   |     | OS-native NSMenu / HMENU       |
|   base/browser/ui/menu/menubar.ts|     | click -> IPC 'vscode:runAction'|
|   – renders <div role="menubar"> |     | -> renderer executes command   |
|   – Alt / mnemonic / arrow keys  |     +--------------------------------+
|   – opens <Menu> on click        |
+----------------+-----------------+
                 |
                 v
+---------------------------------+
| Menu widget (dropdown)           |
|   base/browser/ui/menu/menu.ts   |
|   – ActionBar subclass           |
|   – absolutely-positioned DOM    |
|     inside button container      |
+---------------------------------+
                 |
                 v
          CommandService.executeCommand("workbench.action.files.newUntitledFile")
```

`BrowserTitlebarPart` is the host: it owns `.titlebar-container` split into
left/center/right, and chooses between `installMenubar()` (custom DOM) or
the main-process `Menu.setApplicationMenu` path.

---

## 2. File-by-file breakdown

### 2.1  Native vs custom decision

`src/vs/platform/window/common/window.ts:176-184`

```ts
export type MenuBarVisibility = 'classic' | 'visible' | 'toggle' | 'hidden' | 'compact';

export function getMenuBarVisibility(configurationService: IConfigurationService): MenuBarVisibility {
    const menuBarVisibility = configurationService.getValue<MenuBarVisibility | 'default'>(MenuSettings.MenuBarVisibility);

    if (menuBarVisibility === 'default' || (menuBarVisibility === 'compact' && hasNativeMenu(configurationService)) || (isMacintosh && isNative)) {
        return 'classic';
    }
```

macOS always returns `'classic'` → falls through to the native path. Windows/
Linux/Web use the value directly. `hasNativeMenu()` (line 164) returns true iff
`titleBarStyle === 'native'`.

### 2.2  Title-bar host

`titlebarPart.ts:458-482` creates three flex regions and installs the
custom menubar into the left region:

```ts
this.rootContainer = append(parent, $('.titlebar-container'));
this.leftContent   = append(this.rootContainer, $('.titlebar-left'));
this.centerContent = append(this.rootContainer, $('.titlebar-center'));
this.rightContent  = append(this.rootContainer, $('.titlebar-right'));
...
if (!this.isAuxiliary && !hasNativeMenu(...) && (!isMacintosh || isWeb) &&
    this.currentMenubarVisibility !== 'compact') {
    this.installMenubar();
}
```

```ts
// installMenubar (416-429)
this.customMenubar.value = this.instantiationService.createInstance(CustomMenubarControl);
this.menubar = append(this.leftContent, $('div.menubar'));
this.menubar.setAttribute('role', 'menubar');
this.customMenubar.value.create(this.menubar);
```

Layout is **flexbox** on `.titlebar-container`: icon + menubar in
`.titlebar-left`, title/command-center/breadcrumbs in `.titlebar-center`,
window controls in `.titlebar-right`.

### 2.3  Bridging workbench actions → MenuBar widget

`src/vs/workbench/browser/parts/titlebar/menubarControl.ts:578-630, 751-768`

```ts
this.menubar = this.reinstallDisposables.add(new MenuBar(this.container, this.getMenuBarOptions(), defaultMenuStyles));
...
private getMenuBarOptions(): IMenuBarOptions {
    return {
        enableMnemonics: this.currentEnableMenuBarMnemonics,
        disableAltFocus: this.currentDisableMenuBarAltFocus,
        visibility: this.currentMenubarVisibility,
        actionRunner: this.actionRunner,
        getKeybinding: (action) => this.keybindingService.lookupKeybinding(action.id),
        alwaysOnMnemonics: this.alwaysOnMnemonics,
        compactMode: this.currentCompactMenuMode,
        ...
    };
}
```

Click dispatch is wired here (line 659):

```ts
const newAction = store.add(new Action(
    menuItem.id, mnemonicMenuLabel(title), menuItem.class, menuItem.enabled,
    () => this.commandService.executeCommand(menuItem.id)));
```

So when the user picks "File > New File", `menuItem.id ===
"workbench.action.files.newUntitledFile"` and `ICommandService.executeCommand`
fires the registered handler synchronously.

### 2.4  The MenuBar widget itself

`base/browser/ui/menu/menubar.ts:93-201` — constructor sets `role="menubar"`
and adds container-level key handlers:

```ts
if (event.equals(KeyCode.LeftArrow) || (tabNav && event.equals(KeyCode.Tab | KeyMod.Shift))) {
    this.focusPrevious();
} else if (event.equals(KeyCode.RightArrow) || (tabNav && event.equals(KeyCode.Tab))) {
    this.focusNext();
} else if (event.equals(KeyCode.Escape) && this.isFocused && !this.isOpen) {
    this.setUnfocusedState();
} else if (!this.isOpen && this.options.enableMnemonics && this.mnemonicsInUse && this.mnemonics.has(key)) {
    this.onMenuTriggered(this.mnemonics.get(key)!, false);
}
```

Top-level buttons are built in `push()` (lines 203-315); each attaches its
own `MOUSE_DOWN` (left-click → `onMenuTriggered(index, true)`), `KEY_UP`
(DownArrow/Enter → open), and `MOUSE_ENTER` (switch while another menu is
open).

### 2.5  Alt handling (mnemonic underscores)

`menubar.ts:183-198` — window-level `keydown` listener fires on Alt+letter:

```ts
if (!this.options.enableMnemonics || !e.altKey || e.ctrlKey || e.defaultPrevented) { return; }
const key = e.key.toLocaleLowerCase();
if (!this.mnemonics.has(key)) { return; }
this.mnemonicsInUse = true;
this.updateMnemonicVisibility(true);
this.onMenuTriggered(this.mnemonics.get(key)!, false);
```

Underscore visibility is driven by `DOM.ModifierKeyEmitter` at
`menubar.ts:866-968`: `child.style.textDecoration = (alwaysOn || visible)
? 'underline' : ''`. Label parsing uses `&&` as marker
(`MENU_MNEMONIC_REGEX`); `mnemonicMenuLabel("&&File")` → `<mnemonic>F</mnemonic>ile`.

### 2.6  Submenu dropdown rendering

`menubar.ts:1001-1062` — `showCustomMenu`:

```ts
const menuHolder = $('div.menubar-menu-items-holder', { 'title': '' });
customMenu.buttonElement.classList.add('open');
const titleBoundingRect = customMenu.titleElement.getBoundingClientRect();
...
menuHolder.style.left = `${titleBoundingRect.left * titleBoundingRectZoom}px`;
menuHolder.style.top  = `${titleBoundingRect.bottom * titleBoundingRectZoom}px`;
customMenu.buttonElement.appendChild(menuHolder);
const menuWidget = this.menuDisposables.add(new Menu(menuHolder, customMenu.actions, menuOptions, this.menuStyle));
```

It is **plain DOM, position:absolute, appended as a child of the menu-button
div** — no portal to `document.body`, no Electron popup window. CSS
(`menubar.css` / `menu.css`) gives it `position: absolute` with a high
z-index so it overlays the rest of the workbench.

### 2.7  Native path (macOS, or `titleBarStyle:"native"`)

`platform/menubar/electron-main/menubar.ts:299-395` builds an `electron.Menu`
and calls `Menu.setApplicationMenu(menu)`. Each leaf item (lines 678-687)
registers a `click` callback that calls `runActionInRenderer(...)`, which
sends IPC (lines 798-804):

```ts
activeWindow.sendWhenReady('vscode:runAction', CancellationToken.None,
    { id: invocation.commandId, from: 'menu' });
```

Renderer receiver, `workbench/electron-browser/window.ts:158-177`:

```ts
ipcRenderer.on('vscode:runAction', async (event, ...argsRaw) => {
    ...
    await this.commandService.executeCommand(request.id, ...args);
});
```

Both paths converge on `ICommandService.executeCommand(menuItem.id)`. On
custom (DOM) the closure at `menubarControl.ts:659` executes directly; on
native, one IPC hop first.

### 2.8  Visibility modes

`menubarControl.ts:504-506` reads the setting:

```ts
private get currentMenubarVisibility(): MenuBarVisibility {
    return getMenuBarVisibility(this.configurationService);
}
```

Effect of each value:

- **classic** – let OS draw the menu bar (native path, always on macOS).
- **visible** – custom DOM menubar always shown inside the titlebar.
- **toggle** – hidden until user presses Alt; shows, then hides on
  blur/Escape (via `MenubarState.HIDDEN` / `VISIBLE`).
- **hidden** – never shown (`titlebarPart.ts:371-377` calls
  `uninstallMenubar()`).
- **compact** – menubar collapses into a single hamburger button in the
  Activity Bar; `createOverflowMenu()` in `menubar.ts:317-389` is the
  implementation, and positioning is driven by
  `currentCompactMenuMode` (horizontal/vertical direction relative to
  the activity bar).

---

## 3. Implementation sketch for AgentMux (Rust + CEF + SolidJS)

AgentMux already has a custom title bar drawn by the webview (same model as
VS Code when `titleBarStyle:"custom"`). Two routes:

### Route A — all-DOM inside the custom title bar (recommended default)

Mirrors VS Code's Win/Linux path.

1. `<MenuBar>` SolidJS component inside the titlebar's left region with
   flex layout `[icon][menubar][drag][title][controls]`. Collapse to
   overflow "⋮" then compact as width shrinks (see `menubar.ts:482-563`).
2. A shared `MenuRegistry` of `{id, label, submenu[], accelerator}`;
   each leaf item carries a `commandId` known to the existing command
   service.
3. Dropdown is `position:absolute; z-index:9999` inside the button element
   — not a portal. Close on outside `mousedown` / `blur` / `Escape`.
   Arrow-key focus traversal, Enter/Space to activate.
4. Copy the `HIDDEN|VISIBLE|FOCUSED|OPEN` state machine for Alt handling;
   underline mnemonics on Alt-down and on `mnemonicsInUse`.
5. Click → Solid handler → `invoke("run_command",{id})` → Rust
   `CommandService::execute`.

Pros: one implementation, themeable, works per-CEF-window without sync.
Cons: must re-do accelerators, IME, screen-reader semantics; macOS users
miss the top-of-screen NSMenu.

### Route B — Rust/CEF native menu

Mirrors VS Code's mac/native path via the `muda` crate (HMENU / NSMenu /
GTK). Menu items store a `command_id`; on click, send an IPC message to
the CEF renderer which dispatches through the SolidJS command registry
(exactly the `vscode:runAction` IPC pattern).

Pros: native accelerators + a11y + Linux global menu.
Cons: can't theme, per-window sync needed, can't embed breadcrumbs in
the same bar.

### Hybrid (what VS Code ships)

macOS → Route B, Windows/Linux → Route A, sharing one `MenuRegistry`.
Both paths end at `command_service.execute(menu_item.id)`. VS Code's
`MenuRegistry.appendMenuItem(MenuId.MenubarMainMenu, ...)` at
`menubarControl.ts:48-116` feeds both `CustomMenubarControl` and
`MenubarMainService` off a single declaration — copy that pattern.

---

## 4. Direct links

- https://github.com/microsoft/vscode/blob/main/src/vs/base/browser/ui/menu/menubar.ts
- https://github.com/microsoft/vscode/blob/main/src/vs/base/browser/ui/menu/menu.ts
- https://github.com/microsoft/vscode/blob/main/src/vs/workbench/browser/parts/titlebar/titlebarPart.ts
- https://github.com/microsoft/vscode/blob/main/src/vs/workbench/browser/parts/titlebar/menubarControl.ts
- https://github.com/microsoft/vscode/blob/main/src/vs/platform/menubar/electron-main/menubar.ts
- https://github.com/microsoft/vscode/blob/main/src/vs/platform/window/common/window.ts
- https://github.com/microsoft/vscode/blob/main/src/vs/workbench/electron-browser/window.ts (IPC receiver)
