// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0
//
// SolidJS migration: all Jotai/React types replaced with SolidJS equivalents.

import type { Placement } from "@floating-ui/dom";
import type { Accessor, JSX } from "solid-js";
import type { SignalAtom } from "@/util/util";
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
        blockId: string;
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
        getAboutModalDetails: () => AboutModalDetails;
        getBackendInfo: () => Promise<{ pid?: number; started_at?: string; web_endpoint?: string; version: string }>;
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
        getUpdaterChannel: () => string;
        installAppUpdate: () => void;
        onMenuItemAbout: (callback: () => void) => void;
        updateWindowControlsOverlay: (rect: Dimensions) => void;
        onReinjectKey: (callback: (waveEvent: WaveKeyboardEvent) => void) => void;
        onControlShiftStateUpdate: (callback: (state: boolean) => void) => void;
        openNewWindow: () => Promise<string>;
        closeWindow: (label?: string) => Promise<void>;
        minimizeWindow: () => void;
        maximizeWindow: () => void;
        toggleDevtools: () => void;
        setWindowTransparency: (transparent: boolean, blur: boolean, opacity: number) => void;
        getWindowLabel: () => Promise<string>;
        isMainWindow: () => Promise<boolean>;
        listWindows: () => Promise<string[]>;
        focusWindow: (label: string) => Promise<void>;
        getInstanceNumber: () => Promise<number>;
        getWindowCount: () => Promise<number>;
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
        captureScreenshot(rect: { x: number; y: number; width: number; height: number }): Promise<string>;
        setKeyboardChordMode: () => void;
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
        openWindowAtPosition: (screenX: number, screenY: number) => Promise<string>;
        setDragCursor: () => Promise<void>;
        restoreDragCursor: () => Promise<void>;
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
        viewName?: Accessor<string>;
        viewText?: Accessor<string | HeaderElem[]>;
        preIconButton?: Accessor<IconButtonDecl>;
        endIconButtons?: Accessor<IconButtonDecl[]>;
        blockBg?: Accessor<MetaType>;
        noHeader?: Accessor<boolean>;
        manageConnection?: Accessor<boolean>;
        filterOutNowsh?: Accessor<boolean>;
        showS3?: Accessor<boolean>;
        noPadding?: Accessor<boolean>;
        searchAtoms?: SearchAtoms;
        viewComponent: ViewComponent<any>;
        isBasicTerm?: () => boolean;
        getSettingsMenuItems?: () => ContextMenuItem[];
        giveFocus?: () => boolean;
        keyDownHandler?: (e: WaveKeyboardEvent) => boolean;
        dispose?: () => void;
    }

    type UpdaterStatus = "up-to-date" | "checking" | "downloading" | "ready" | "error" | "installing";

    interface Dimensions {
        width: number;
        height: number;
        left: number;
        top: number;
    }

    type TypeAheadModalType = { [key: string]: boolean };

    interface AboutModalDetails {
        version: string;
        buildTime: number;
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

    interface AbstractWshClient {
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
}

export {};
