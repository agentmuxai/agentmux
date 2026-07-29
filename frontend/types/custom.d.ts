// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// SolidJS migration: all Jotai/React types replaced with SolidJS equivalents.

import type { Placement } from "@floating-ui/dom";
import type { Accessor, JSX } from "solid-js";
import type { SignalAtom } from "@/util/util";
import type { PaneVoiceHandle } from "@/app/hook/useVoiceInput";
import type * as rxjs from "rxjs";

declare global {
    // All atoms are now SolidJS Accessors (call as function to read reactive value).
    // For writable atoms use SignalAtom (also callable, plus ._set()).
    type GlobalAtomsType = {
        clientId: Accessor<string>;
        client: Accessor<Client>;
        uiContext: Accessor<UIContext>;
        waveWindow: Accessor<WaveWindow>;
        workspace: Accessor<Workspace>;
        fullConfigAtom: Accessor<FullConfigType>;
        settingsAtom: Accessor<SettingsType>;
        hasCustomAIPresetsAtom: Accessor<boolean>;
        tabAtom: Accessor<Tab>;
        staticTabId: Accessor<string>;
        activeTabId: Accessor<string>;
        isFullScreen: Accessor<boolean>;
        controlShiftDelayAtom: Accessor<boolean>;
        prefersReducedMotionAtom: Accessor<boolean>;
        updaterStatusAtom: Accessor<UpdaterStatus>;
        typeAheadModalAtom: Accessor<TypeAheadModalType>;
        modalOpen: Accessor<boolean>;
        allConnStatus: Accessor<ConnStatus[]>;
        flashErrors: Accessor<FlashErrorType[]>;
        notifications: Accessor<NotificationType[]>;
        notificationPopoverMode: Accessor<boolean>;
        reinitVersion: Accessor<number>;
        isTermMultiInput: Accessor<boolean>;
        backendStatusAtom: Accessor<"connecting" | "running" | "crashed">;
        lanInstancesAtom: Accessor<LanInstance[]>;
    };

    type LanInstance = {
        instance_id: string;
        hostname: string;
        version: string;
        address: string;
        port: number;
        agents: string[];
        first_seen: number;
        last_seen: number;
    };

    // Editor file-tree row. Spec: specs/SPEC_EDITOR_FILE_TREE_2026-05-26.md
    type DirEntry = {
        name: string;
        is_dir: boolean;
        is_symlink: boolean;
        size?: number;
        mtime?: number; // unix millis
    };

    type WritableWaveObjectAtom<T extends WaveObj> = SignalAtom<T>;

    type ThrottledValueAtom<T> = SignalAtom<T>;

    type AtomWithThrottle<T> = {
        currentValueAtom: Accessor<T>;
        throttledValueAtom: ThrottledValueAtom<T>;
    };

    type DebouncedValueAtom<T> = SignalAtom<T>;

    type AtomWithDebounce<T> = {
        currentValueAtom: Accessor<T>;
        debouncedValueAtom: DebouncedValueAtom<T>;
    };

    type TabLayoutData = {
        /** The currently-rendered block for this leaf. Always kept in sync
         *  with `activeBlockId` when `blockStack` is non-empty. */
        blockId: string;
        /** In-pane tabs (SPEC_PANE_TAB_STRIP_AGENT_TERMINAL_2026_07_20.md
         *  §4.3): every blockId hosted by this leaf, ordered; absent/empty
         *  means "no stack, just `blockId`" — 100% back-compat with every
         *  layout written before this field existed. */
        blockStack?: string[];
        /** The active member of `blockStack`. Unused when `blockStack` is
         *  absent/empty. */
        activeBlockId?: string;
    };

    type AgentMuxInitOpts = {
        tabId: string;
        clientId: string;
        windowId: string;
        activate: boolean;
        primaryTabStartup?: boolean;
    };

    type AppApi = {
        getAuthKey(): string;
        getIsDev(): boolean;
        getCursorPoint: () => { x: number; y: number };
        getPlatform: () => NodeJS.Platform;
        getEnv: (varName: string) => string;
        getUserName: () => string;
        getHostName: () => string;
        getDataDir: () => string;
        getConfigDir: () => string;
        getUserHomeDir: () => string;
        getAboutModalDetails: () => AboutModalDetails;
        getBackendInfo: () => Promise<{ pid?: number; started_at?: string; web_endpoint?: string; version: string; pending_migrations?: number }>;
        restartBackend: () => Promise<void>;
        getDocsiteUrl: () => string;
        getZoomFactor: () => number;
        showContextMenu: (workspaceId: string, menu?: NativeContextMenuItem[], position?: { x: number; y: number }) => void;
        onContextMenuClick: (callback: (id: string) => void) => void;
        onNavigate: (callback: (url: string) => void) => void;
        onIframeNavigate: (callback: (url: string) => void) => void;
        downloadFile: (path: string) => void;
        openExternal: (url: string) => void;
        onFullScreenChange: (callback: (isFullScreen: boolean) => void) => void;
        onZoomFactorChange: (callback: (zoomFactor: number) => void) => void;
        setZoomFactor: (zoomFactor: number) => void;
        onUpdaterStatusChange: (callback: (status: UpdaterStatus) => void) => void;
        getUpdaterStatus: () => UpdaterStatus;
        getUpdaterVersion: () => string | null;
        getUpdaterChannel: () => string;
        installAppUpdate: () => void;
        onMenuItemAbout: (callback: () => void) => void;
        updateWindowControlsOverlay: (rect: Dimensions) => void;
        onReinjectKey: (callback: (waveEvent: WaveKeyboardEvent) => void) => void;
        onControlShiftStateUpdate: (callback: (state: boolean) => void) => void;
        openNewWindow: () => Promise<string>;
        openNewWindowWithView: (view: string, meta?: Record<string, unknown>) => Promise<string>;
        closeWindow: (label?: string) => Promise<void>;
        minimizeWindow: () => void;
        maximizeWindow: () => void;
        toggleDevtools: () => void;
        inspectElementAt: (x: number, y: number) => void;
        setWindowTransparency: (transparent: boolean, blur: boolean, opacity: number) => void;
        setWindowOpacity: (label: string, opacity: number) => Promise<void>;
        getWindowOpacity: (label: string) => Promise<number>;
        getWindowLabel: () => Promise<string>;
        isMainWindow: () => Promise<boolean>;
        registerBackendWindow: (label: string, windowId: string) => void;
        listWindows: () => Promise<string[]>;
        /** Like listWindows but returns `[{label, windowId}]` pairs.
         *  `windowId` is null for windows that haven't yet completed
         *  the registerBackendWindow round-trip. Used by InstancePanel
         *  to look up per-window backend Window records (display name
         *  in meta, workspace fallback, etc.) without an extra RPC. */
        listWindowInstances: () => Promise<Array<{ label: string; windowId: string | null }>>;
        /** OS-reported double-click interval in milliseconds. Used by
         *  InstancePanel to defer single-click focus past the user's
         *  configured threshold so double-click-to-rename works for
         *  slow double-clickers (Win32 GetDoubleClickTime, default
         *  500ms, user-configurable). Falls back to 500ms on non-
         *  Windows. */
        getDoubleClickTime: () => Promise<number>;
        focusWindow: (label: string) => Promise<void>;
        getInstanceNumber: () => Promise<number>;
        createWorkspace: () => void;
        switchWorkspace: (workspaceId: string) => void;
        deleteWorkspace: (workspaceId: string) => void;
        setActiveTab: (tabId: string) => void;
        createTab: () => void;
        closeTab: (workspaceId: string, tabId: string) => void;
        setWindowInitStatus: (status: "ready" | "wave-ready") => void;
        onAgentMuxInit: (callback: (initOpts: AgentMuxInitOpts) => void) => void;
        sendLog: (log: string) => void;
        sendLogStructured: (level: string, module: string, message: string, data: Record<string, any> | null) => void;
        onQuicklook: (filePath: string) => void;
        openNativePath(filePath: string): void;
        revealInFileExplorer(filePath: string): void;
        /** Native "open file" dialog. Resolves to the chosen absolute path,
         *  or null if the user cancelled. See
         *  docs/specs/SPEC_MEDIA_PANE_2026_07_26.md. */
        showOpenFileDialog(): Promise<string | null>;
        captureScreenshot(rect: { x: number; y: number; width: number; height: number }): Promise<string>;
        setKeyboardChordMode: () => void;
        openAgent: (agentId: string) => Promise<void>;
        openClaudeCodeAuth: () => Promise<void>;
        getClaudeCodeAuth: () => Promise<{ connected: boolean; email?: string; expires_at?: number }>;
        disconnectClaudeCode: () => Promise<void>;
        detectInstalledClis: () => Promise<CliDetectionResult[]>;
        getProviderConfig: () => Promise<ProviderConfig>;
        saveProviderConfig: (config: ProviderConfig) => Promise<void>;
        getProviderInstallInfo: (provider: string) => Promise<ProviderInstallInfo>;
        setProviderAuth: (provider: string, token: string) => Promise<void>;
        clearProviderAuth: (provider: string) => Promise<void>;
        getProviderAuthStatus: (provider: string) => Promise<ProviderAuthStatus>;
        checkCliAuthStatus: (provider: string, cliPath?: string) => Promise<CliAuthStatus>;
        installCli: (provider: string) => Promise<CliInstallResult>;
        getCliPath: (provider: string) => Promise<string | null>;
        checkNodejsAvailable: () => Promise<NodejsStatus>;
        ensureAuthDir: (providerId: string) => Promise<string>;
        runCliLogin: (cliPath: string, loginArgs: string[], authEnv: Record<string, string>, requiresTty?: boolean) => Promise<string | null>;
        cancelCliLogin: () => Promise<void>;
        seedProviderAuthFromGlobal: (providerId: string, configDir?: string) => Promise<{ seeded: boolean; status: string; expiresAt?: number | null }>;
        openLoginTerminal: (cliPath: string, loginArgs: string[], authEnv: Record<string, string>) => Promise<{ opened: boolean }>;
        listen: (event: string, callback: (event: any) => void) => Promise<() => void>;
        startCrossDrag: (
            dragType: "pane" | "tab",
            sourceWindow: string,
            sourceWorkspaceId: string,
            sourceTabId: string,
            payload: { blockId?: string; tabId?: string }
        ) => Promise<string>;
        updateCrossDrag: (dragId: string, screenX: number, screenY: number) => Promise<string | null>;
        completeCrossDrag: (
            dragId: string,
            targetWindow: string | null,
            screenX: number,
            screenY: number
        ) => Promise<void>;
        cancelCrossDrag: (dragId: string) => Promise<void>;
        /** Cold-path tear-off / new top-level. `width`/`height` set the
         *  outer window dimensions when matching the source window's
         *  size on tab tear-off; omit for the historical default.
         *  `tabAnchorX`/`tabAnchorY` are the screen point where the
         *  user grabbed the tab — backend places the new window so its
         *  first tab lands at that point (Chrome-style no-teleport
         *  handoff). Omit for cursor-centered fallback. */
        openWindowAtPosition: (
            screenX: number,
            screenY: number,
            workspaceId?: string,
            width?: number,
            height?: number,
            tabAnchorX?: number,
            tabAnchorY?: number,
        ) => Promise<string>;
        /** Phase 6 — promote a pre-warmed pool window for tear-off.
         *  Returns the destination window label on success. Throws if
         *  the pool is empty (caller should fall back to
         *  openWindowAtPosition for the cold path). `width`/`height`
         *  resize the promoted window to match the source frame on
         *  tab tear-off; omit to keep the pool default.
         *  `tabAnchorX`/`tabAnchorY` are the screen point where the
         *  user grabbed the tab — backend places the new window so its
         *  first tab lands at that point. Omit for cursor-centered. */
        tearOffPoolPromote: (
            workspaceId: string,
            screenX: number,
            screenY: number,
            width?: number,
            height?: number,
            tabAnchorX?: number,
            tabAnchorY?: number,
        ) => Promise<string>;
        /** Tear-off Phase 2 Win32 SC_MOVE handshake. Call AFTER
         *  TearOffTab + openWindowAtPosition; this hands cursor capture
         *  to the new window so it follows the mouse like a Chrome
         *  torn-off tab. See SPEC_TAB_TEAR_OFF_SIZE_PRESERVATION §4.2. */
        tearOffSCMoveHandshake: (args: {
            sourceWindowLabel: string;
            destWindowLabel: string;
            cursorX: number;
            cursorY: number;
            /** Phase 4 — drives the WH_MOUSE_LL hook for cross-window
             *  merge detection. When all three are non-empty, the host
             *  arms the hook before posting SC_MOVE; on mouseup the
             *  candidate window receives `tearoff:merge` (or the source
             *  receives `tearoff:standalone` if no candidate). */
            tabId?: string;
            sourceWsId?: string;
            destWsId?: string;
            /** Phase 5 — tab's index in the source workspace at
             *  tear-off time. Used by cancel-back (ESC or drop on
             *  source strip) to restore at the original position.
             *  When `wasPinned` is true, this is the index inside
             *  `pinnedtabids`; otherwise inside `tabids`. */
            originalTabIndex?: number;
            /** Phase 5 — was the tab pinned in its source workspace?
             *  Threaded through to the cancel-back payload so the
             *  backend can restore into pinnedtabids vs tabids and
             *  preserve pinned status. */
            wasPinned?: boolean;
        }) => Promise<{ handshakeMs: number; totalMs: number }>;
        /** Close a specific AgentMux window by label. Used by Phase 4
         *  merge to clean up the dragged window after its tab is moved
         *  into the destination workspace. */
        closeWindowByLabel: (label: string) => Promise<void>;
        /** Cross-window tab remount (SPEC_CROSS_WINDOW_TAB_REMOUNT §4.1):
         *  arm the global mouse hook for an ordinary in-strip tab drag.
         *  On release over another AgentMux window, that window receives
         *  `tabdrag:merge-direct`. No-op on non-Windows. */
        startTabDragTracking: (args: {
            sourceWindowLabel: string;
            tabId: string;
            sourceWsId: string;
            isLastTab: boolean;
        }) => Promise<void>;
        /** Belt-and-suspenders hook teardown at dragend — the hook
         *  self-uninstalls on mouseup/ESC, but a leaked WH_MOUSE_LL hook
         *  degrades every mouse event on the system, so dragend calls
         *  this unconditionally. Idempotent. */
        stopTabDragTracking: () => Promise<void>;
        setDragCursor: () => Promise<void>;
        restoreDragCursor: () => Promise<void>;
        releaseDragCapture: () => Promise<void>;
        getMouseButtonState: () => Promise<boolean>;
        setJsDragActive: (active: boolean) => Promise<void>;
        /** SPEC_NATIVE_POINTER_DRAG_TEAROFF_2026_07_28 — begin a native
         *  pointer-capture window drag for `label`. `grabOffsetX/Y` are the
         *  cursor's offset from the window's top-left (physical screen px)
         *  at engage time. Windows-only; no-op elsewhere. */
        engageNativeWindowDrag: (label: string, grabOffsetX: number, grabOffsetY: number) => Promise<void>;
        /** Reposition the engaged window per pointermove. Cursor position
         *  in physical screen px. No-op if no drag is engaged. */
        updateNativeWindowDrag: (screenX: number, screenY: number) => Promise<void>;
        /** End the native window drag gesture. Idempotent. */
        endNativeWindowDrag: () => Promise<void>;
        /** Trigger on-demand migration run (maintenance panel retry).
         *  Returns immediately; progress arrives via `upgrade:migration-event` CEF events. */
        runMigrations: () => Promise<{ started: boolean }>;
        /** Trigger on-demand saga log vacuum. Returns rows deleted. */
        runSagaVacuum: () => Promise<{ rows_deleted: number }>;
    };

    type NativeContextMenuItem = {
        id: string;
        label: string;
        role?: string;
        type?: "separator" | "normal" | "submenu" | "checkbox" | "radio";
        submenu?: NativeContextMenuItem[];
        checked?: boolean;
        visible?: boolean;
        enabled?: boolean;
        sublabel?: string;
        /** CSS color for an inline swatch square rendered before the label. */
        swatchColor?: string;
    };

    type ContextMenuItem = {
        label?: string;
        type?: "separator" | "normal" | "submenu" | "checkbox" | "radio";
        role?: string;
        click?: () => void;
        submenu?: ContextMenuItem[];
        checked?: boolean;
        visible?: boolean;
        enabled?: boolean;
        sublabel?: string;
        /** CSS color for an inline swatch square rendered before the label. */
        swatchColor?: string;
    };

    type KeyPressDecl = {
        mods: {
            Cmd?: boolean;
            Option?: boolean;
            Shift?: boolean;
            Ctrl?: boolean;
            Alt?: boolean;
            Meta?: boolean;
        };
        key: string;
        keyType: string;
    };

    type SubjectWithRef<T> = rxjs.Subject<T> & { refCount: number; release: () => void };

    type HeaderElem =
        | IconButtonDecl
        | ToggleIconButtonDecl
        | HeaderText
        | HeaderInput
        | HeaderDiv
        | HeaderTextButton
        | ConnectionButton
        | MenuButton;

    type IconButtonCommon = {
        icon: string | JSX.Element;
        iconColor?: string;
        iconSpin?: boolean;
        className?: string;
        title?: string;
        disabled?: boolean;
        noAction?: boolean;
    };

    type IconButtonDecl = IconButtonCommon & {
        elemtype: "iconbutton";
        click?: (e: MouseEvent) => void;
        longClick?: (e: MouseEvent) => void;
    };

    type ToggleIconButtonDecl = IconButtonCommon & {
        elemtype: "toggleiconbutton";
        active: SignalAtom<boolean>;
    };

    type HeaderTextButton = {
        elemtype: "textbutton";
        text: string;
        className?: string;
        title?: string;
        onClick?: (e: MouseEvent) => void;
    };

    type HeaderText = {
        elemtype: "text";
        text: string;
        ref?: { current: HTMLDivElement | null };
        className?: string;
        noGrow?: boolean;
        onClick?: (e: MouseEvent) => void;
    };

    type HeaderInput = {
        elemtype: "input";
        value: string;
        className?: string;
        isDisabled?: boolean;
        ref?: { current: HTMLInputElement | null };
        onChange?: (e: Event) => void;
        onKeyDown?: (e: KeyboardEvent) => void;
        onFocus?: (e: FocusEvent) => void;
        onBlur?: (e: FocusEvent) => void;
    };

    type HeaderDiv = {
        elemtype: "div";
        className?: string;
        children: HeaderElem[];
        onMouseOver?: (e: MouseEvent) => void;
        onMouseOut?: (e: MouseEvent) => void;
        onClick?: (e: MouseEvent) => void;
    };

    type ConnectionButton = {
        elemtype: "connectionbutton";
        icon: string;
        text: string;
        iconColor: string;
        onClick?: (e: MouseEvent) => void;
        connected: boolean;
    };

    type MenuItem = {
        label: string;
        icon?: string | JSX.Element;
        subItems?: MenuItem[];
        onClick?: (e: MouseEvent) => void;
        divider?: boolean;
        // Radio/checkbox indicator. When set, FlyoutMenu renders a
        // check icon in the icon slot for true and a blank-width
        // spacer for false (so radio groups stay aligned). When
        // unset, the regular `icon` field renders.
        checked?: boolean;
        // Pre-formatted keyboard shortcut hint shown right-aligned
        // (e.g. "Ctrl+P" or "⌘T"). Not shown on items that have subItems.
        shortcut?: string;
    };

    type MenuButtonProps = {
        items: MenuItem[];
        className?: string;
        text: string;
        title?: string;
        menuPlacement?: Placement;
    };

    type MenuButton = {
        elemtype: "menubutton";
    } & MenuButtonProps;

    type SearchAtoms = {
        searchValue: SignalAtom<string>;
        resultsIndex: SignalAtom<number>;
        resultsCount: SignalAtom<number>;
        isOpen: SignalAtom<boolean>;
        regex?: SignalAtom<boolean>;
        caseSensitive?: SignalAtom<boolean>;
        wholeWord?: SignalAtom<boolean>;
    };

    // SolidJS component props for block views
    declare type ViewComponentProps<T extends ViewModel = ViewModel> = {
        blockId: string;
        blockRef: { current: HTMLDivElement | null };
        contentRef: { current: HTMLDivElement | null };
        model: T;
    };

    // A SolidJS function component
    declare type ViewComponent<T extends ViewModel = ViewModel> = (props: ViewComponentProps<T>) => JSX.Element;

    type ViewModelClass = new (blockId: string, nodeModel: BlockNodeModel) => ViewModel;

    interface ViewModel {
        viewType: string;
        viewIcon?: Accessor<string | IconButtonDecl>;
        /** When set, overrides the FA icon in the block header with a favicon <img>.
         *  The consumer (blockframe) falls back to viewIcon if this is empty string. */
        viewFaviconUrl?: Accessor<string>;
        viewName?: Accessor<string>;
        /** When provided, the header name becomes an inline editable text field. */
        setViewName?: (name: string) => Promise<void>;
        viewText?: Accessor<string | HeaderElem[]>;
        preIconButton?: Accessor<IconButtonDecl>;
        endIconButtons?: Accessor<IconButtonDecl[]>;
        blockBg?: Accessor<MetaType>;
        noHeader?: Accessor<boolean>;
        manageConnection?: Accessor<boolean>;
        showS3?: Accessor<boolean>;
        noPadding?: Accessor<boolean>;
        searchAtoms?: SearchAtoms;
        viewComponent: ViewComponent<any>;
        isBasicTerm?: () => boolean;
        getSettingsMenuItems?: () => ContextMenuItem[];
        getBodyContextMenuItems?: () => ContextMenuItem[];
        giveFocus?: () => boolean;
        keyDownHandler?: (e: WaveKeyboardEvent) => boolean;
        dispose?: () => void;
        /** Views that support voice input expose a handle accessor. Called
         *  by BlockFrame_Header (to render the mic button) and by the
         *  Ctrl+Shift+V global hotkey to retarget the voice session. */
        voiceHandle?: () => PaneVoiceHandle;
    }

    type UpdaterStatus = "up-to-date" | "checking" | "available" | "downloading" | "ready" | "error" | "installing";

    interface Dimensions {
        width: number;
        height: number;
        left: number;
        top: number;
    }

    type TypeAheadModalType = { [key: string]: boolean };

    interface AboutModalDetails {
        version: string;
        gitHash?: string;
        buildTime: number;
        buildLabel?: string;
        channel?: string;
        platform?: string;
        arch?: string;
    }

    type BlockComponentModel = {
        openSwitchConnection?: () => void;
        viewModel: ViewModel;
    };

    type ConnStatusType = "connected" | "connecting" | "disconnected" | "error" | "init";

    interface SuggestionBaseItem {
        label: string;
        value: string;
        icon?: string | JSX.Element;
    }

    interface SuggestionConnectionItem extends SuggestionBaseItem {
        status: ConnStatusType;
        iconColor: string;
        onSelect?: (_: string) => void;
        current?: boolean;
    }

    interface SuggestionConnectionScope {
        headerText?: string;
        items: SuggestionConnectionItem[];
    }

    type SuggestionsType = SuggestionConnectionItem | SuggestionConnectionScope;

    type MarkdownResolveOpts = {
        connName: string;
        baseDir: string;
    };

    type FlashErrorType = {
        id: string;
        icon: string;
        title: string;
        message: string;
        expiration: number;
    };

    export type NotificationActionType = {
        label: string;
        actionKey: string;
        rightIcon?: string;
        color?: "green" | "grey";
        disabled?: boolean;
    };

    export type NotificationType = {
        id?: string;
        icon: string;
        title: string;
        message: string;
        timestamp: string;
        expiration?: number;
        hidden?: boolean;
        actions?: NotificationActionType[];
        persistent?: boolean;
        type?: "error" | "update" | "info" | "warning";
    };

    interface AbstractRpcClient {
        recvRpcMessage(msg: RpcMessage): void;
    }

    type ClientRpcEntry = {
        reqId: string;
        startTs: number;
        command: string;
        msgFn: (msg: RpcMessage) => void;
    };

    type TimeSeriesMeta = {
        name?: string;
        color?: string;
        label?: string;
        maxy?: string | number;
        miny?: string | number;
        decimalPlaces?: number;
    };

    interface SuggestionRequestContext {
        widgetid: string;
        reqnum: number;
        dispose?: boolean;
    }

    type SuggestionsFnType = (query: string, reqContext: SuggestionRequestContext) => Promise<FetchSuggestionsResponse>;

    type CliDetectionResult = {
        provider: string;
        installed: boolean;
        path: string | null;
        version: string | null;
    };

    type ProviderConfig = {
        default_provider: string;
        providers: Record<string, ProviderSettings>;
        setup_complete: boolean;
    };

    type ProviderSettings = {
        cli_path: string | null;
        auth_token: string | null;
        auth_status: string;
        output_format: string;
        extra_args: string[];
    };

    type ProviderInstallInfo = {
        provider: string;
        install_command: string;
        docs_url: string;
    };

    type ProviderAuthStatus = {
        provider: string;
        status: string;
        error: string | null;
    };

    type CliAuthStatus = {
        logged_in: boolean;
        auth_method: string | null;
        api_provider: string | null;
        email: string | null;
        subscription_type: string | null;
    };

    type CliInstallResult = {
        provider: string;
        cli_path: string;
        version: string;
        already_installed: boolean;
    };

    type NodejsStatus = {
        available: boolean;
        version: string | null;
        npm_available: boolean;
        npm_version: string | null;
        path: string | null;
    };

    type DraggedFile = {
        uri: string;
        absParent: string;
        relName: string;
        isDir: boolean;
    };

    type ErrorButtonDef = {
        text: string;
        onClick: () => void;
    };

    type ErrorMsg = {
        status: string;
        text: string;
        level?: "error" | "warning";
        buttons?: Array<ErrorButtonDef>;
        closeAction?: () => void;
        showDismiss?: boolean;
    };

    type AIMessage = {
        messageid: string;
        parts: AIMessagePart[];
    };

    type AIMessagePart =
        | {
              type: "text";
              text: string;
          }
        | {
              type: "file";
              mimetype: string;
              filename?: string;
              data?: string;
              url?: string;
              size?: number;
              previewurl?: string;
          };

    // SolidJS Block node model (replaces React-specific BlockNodeModel references)
    interface BlockNodeModel {
        blockId: string;
        isFocused: Accessor<boolean>;
        focusNode: () => void;
        disablePointerEvents: Accessor<boolean>;
        innerRect?: Accessor<{ width: string; height: string }>;
    }

    // Window extensions — eliminates (window as any) casts throughout the codebase
    interface Window {
        // Platform API (set by CEF/Tauri bootstrap)
        api: AppApi;
        globalAtoms: any; // GlobalAtomsType has a pre-existing type mismatch — fix separately

        // Debug utilities (exposed for console access)
        RpcApi: any;
        WOS: any;
        TabRpcClient: any;
        globalWS: any;
        modalsModel: any;
        term: any;
        debugLog: (...args: any[]) => void;
        countersPrint: () => void;
        countersClear: () => void;
        getLayoutModelForStaticTab: (tabId: string) => any;
        isFullScreen: boolean;

        // Notification helpers
        pushNotification: (notif: NotificationType) => void;
        pushFlashError: (error: FlashErrorType) => void;
        removeNotificationById: (id: string) => void;

        // Bootstrap flags (set before app init)
        __AGENTMUX_IPC_PORT__?: number;
        __AGENTMUX_IPC_TOKEN__?: string;
        __WAVE_SERVER_WS_ENDPOINT__?: string;
        __WAVE_SERVER_WEB_ENDPOINT__?: string;
        __startupPerfStart?: number;

        // Phase B.7.3.1 — launcher typed-event dispatcher.
        // Installed by `frontend/util/launcher-events.ts`; called
        // by the host's CEF JS bridge once per top-level renderer
        // per launcher event.
        __agentmux_launcher_event?: (evt: { event: string; version: number; [k: string]: unknown }) => void;

        // Phase E.2c.5b — srv typed-event dispatcher.
        // Installed by `frontend/util/srv-events.ts`; called by the
        // host's CEF JS bridge (`agentmux-cef/src/srv_event_bridge.rs`)
        // once per top-level renderer per srv event.
        __agentmux_srv_event?: (evt: { event: string; version: number; [k: string]: unknown }) => void;
    }
}

export {};
