# AgentMux Mobile — Architecture Specification

**Date:** 2026-03-11
**Version:** 1.1
**Status:** Draft
**Repository:** [agentmuxai/agentmux-mobile](https://github.com/agentmuxai/agentmux-mobile) (private)

## Overview

AgentMux Mobile is a companion app for AgentMux, built with **Flutter** targeting **Android** and **iOS**. It provides terminal access, AI agent interaction, and session management — connecting to a running `agentmuxsrv-rs` backend over WebSocket.

The mobile app is a **thin client** — it does not embed the Rust backend or run PTY sessions locally. Instead it connects to a local or remote AgentMux backend, consuming the same WebSocket RPC API that the desktop Tauri frontend uses today.

---

## Goals

1. **Terminal access** — Full terminal emulation on mobile/tablet, connect to existing AgentMux sessions
2. **AI agent pane** — Interact with Claude and other AI agents from mobile
3. **Session continuity** — See and resume sessions started on desktop
4. **Android native** — Touch-optimized terminal and AI experience, phone + tablet
5. **iOS native** — First-class iPhone and iPad experience
6. **Code reuse** — Share the Rust backend protocol; no Go, no Electron, no duplicate logic

## Non-Goals (v1)

- Local PTY on mobile (no shell on Android/iOS)
- Full parity with desktop (no code editor, no sysinfo, no drag-and-drop layout)
- macOS/Linux/Windows desktop (Tauri desktop app covers these)
- Offline mode

---

## Architecture

### High-Level

```
┌─────────────────────────────────────────────────────────┐
│                   AgentMux Mobile (Flutter)              │
│                   Android + iOS                          │
│                                                         │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌────────┐  │
│  │ Terminal  │  │ AI Agent │  │ Sessions │  │Settings│  │
│  │  View    │  │  View    │  │  List    │  │        │  │
│  └────┬─────┘  └────┬─────┘  └────┬─────┘  └───┬────┘  │
│       │              │             │             │       │
│  ┌────┴──────────────┴─────────────┴─────────────┴───┐  │
│  │              State Management (Riverpod)           │  │
│  └────────────────────────┬──────────────────────────┘  │
│                           │                             │
│  ┌────────────────────────┴──────────────────────────┐  │
│  │           WebSocket RPC Client (Dart)              │  │
│  │     (same protocol as desktop Tauri frontend)      │  │
│  └────────────────────────┬──────────────────────────┘  │
└───────────────────────────┼─────────────────────────────┘
                            │ WSS
                            ▼
              ┌──────────────────────────┐
              │   agentmuxsrv-rs (Rust)  │
              │   (running on desktop    │
              │    or remote server)     │
              └──────────────────────────┘
```

### Why Thin Client?

The desktop app (Tauri) spawns `agentmuxsrv-rs` as a sidecar — the backend manages PTY sessions, SQLite storage, file I/O, AI streaming, and config. The frontend communicates entirely over WebSocket RPC.

This means a mobile client can connect to the same backend without embedding any Rust code. Benefits:

- **No flutter_rust_bridge complexity** — pure Dart WebSocket client
- **No PTY on mobile** — terminals are remote by design (connecting to desktop/server sessions)
- **Session continuity** — mobile sees the same blocks, tabs, and workspaces as desktop
- **Smaller app size** — no Rust binary bundled (~5 MB saved)

If we later want local Rust logic (e.g., offline config, local AI inference), `flutter_rust_bridge` can be added incrementally.

---

## Platform Targets

| Platform | Status | Notes |
|----------|--------|-------|
| **Android** | v1 | Phone + tablet, API 26+ (Android 8.0+) |
| **iOS** | v1 | iPhone + iPad, iOS 16+ |
| **macOS** | Future | Flutter desktop supports it, low effort from iOS codebase |
| **Windows/Linux** | N/A | Desktop app (Tauri) covers these |

---

## Tech Stack

| Layer | Technology | Rationale |
|-------|-----------|-----------|
| **Framework** | Flutter 3.x + Dart 3.x | Single codebase for Android + iOS |
| **State** | Riverpod | Compile-time safe, modular, excellent for async streams |
| **Terminal** | [xterm.dart](https://github.com/TerminalStudio/xterm.dart) | Mature Flutter terminal emulator, mobile-optimized |
| **Networking** | `web_socket_channel` + custom RPC | Match agentmuxsrv-rs WebSocket protocol |
| **SSH** | [dartssh2](https://github.com/TerminalStudio/dartssh2) | Pure Dart SSH client (future: direct SSH without backend) |
| **Storage** | `shared_preferences` + `flutter_secure_storage` | Connection configs, auth tokens (Keychain on iOS, Keystore on Android) |
| **Navigation** | `go_router` | Declarative routing |
| **QR** | `mobile_scanner` | Camera-based QR code scanning for pairing |

---

## Backend Protocol

The mobile app speaks the same WebSocket RPC protocol as the Tauri frontend. No new backend endpoints needed.

### Connection Flow

```
1. User enters backend URL (e.g., 192.168.1.50:1730 or remote host)
2. App connects to GET /ws?authkey=<key>
3. WebSocket upgrades, RPC channel established
4. App calls getfullconfig to load settings, workspaces, tabs
5. App subscribes to events (eventsub) for real-time updates
6. Terminal panes send controllerinput, receive event streams
```

### Key RPC Commands (Mobile Subset)

**Session Management:**
| Command | Purpose |
|---------|---------|
| `getfullconfig` | Load all settings, workspaces, widgets |
| `workspace.ListWorkspaces` | List available workspaces |
| `workspace.GetWorkspace` | Get workspace details (tabs, layout) |
| `client.GetTab` | Get tab with block IDs |
| `object.GetObject` | Get block metadata (view type, controller) |
| `blockinfo` | Get block runtime info |
| `blockslist` | List blocks by criteria |

**Terminal:**
| Command | Purpose |
|---------|---------|
| `controllerinput` | Send keystrokes to PTY (data, signal, resize) |
| `controllerresync` | Reconnect to existing PTY session |
| `eventsub` | Subscribe to terminal output events |
| `object.CreateBlock` | Create new terminal block |

**AI Agent:**
| Command | Purpose |
|---------|---------|
| `aisendmessage` | Send message to AI (streaming response) |
| `getwaveaichat` | Load chat history |
| `waveaitoolapprove` | Approve tool execution |
| `streamwaveai` | Stream AI responses |

**Configuration:**
| Command | Purpose |
|---------|---------|
| `setconfig` | Update settings |
| `setmeta` | Update block/object metadata |

### Authentication

The backend uses an `authkey` (random token generated at startup). The mobile app needs this key to connect. Options:

1. **QR code pairing** — Desktop shows QR with `ws://host:port?authkey=xxx`, mobile scans
2. **Manual entry** — User types host + authkey
3. **mDNS discovery** — Auto-discover AgentMux instances on local network (future)

---

## Features (v1)

### 1. Connection Manager

The entry point of the app — manage connections to AgentMux backends.

```
┌─────────────────────────────────┐
│  AgentMux Mobile                │
│                                 │
│  ┌───────────────────────────┐  │
│  │  My Desktop               │  │
│  │    192.168.1.50:1730      │  │
│  │    Connected - 3 sessions │  │
│  └───────────────────────────┘  │
│                                 │
│  ┌───────────────────────────┐  │
│  │  Dev Server               │  │
│  │    dev.example.com:1730   │  │
│  │    Disconnected           │  │
│  └───────────────────────────┘  │
│                                 │
│  [ + Add Connection ]           │
│  [ Scan QR Code     ]          │
│                                 │
└─────────────────────────────────┘
```

- Save multiple backend connections (host, port, authkey)
- Store credentials in platform secure storage (iOS Keychain / Android Keystore)
- Show connection status, session count
- Auto-reconnect on network change

### 2. Session Browser

Browse workspaces, tabs, and blocks from the connected backend.

```
┌──────────────────────────────────┐
│  < My Desktop                    │
│                                  │
│  Workspace: Default              │
│                                  │
│  ┌────────────────────────────┐  │
│  │  Terminal — zsh             │  │
│  │    /home/user/project      │  │
│  │    Last active: 2m ago     │  │
│  └────────────────────────────┘  │
│                                  │
│  ┌────────────────────────────┐  │
│  │  Agent — Claude             │  │
│  │    "Fix the auth bug..."   │  │
│  │    Active - streaming      │  │
│  └────────────────────────────┘  │
│                                  │
│  ┌────────────────────────────┐  │
│  │  Terminal — ssh prod        │  │
│  │    root@prod-server        │  │
│  │    Last active: 15m ago    │  │
│  └────────────────────────────┘  │
│                                  │
│  [ + New Terminal ] [ + New Agent ] │
└──────────────────────────────────┘
```

- List all blocks across tabs, grouped by workspace
- Show block type (terminal/agent), status, last activity
- Tap to open/resume session
- Create new terminal or agent blocks

### 3. Terminal View

Full terminal emulation using `xterm.dart`.

**Features:**
- Render terminal output from PTY stream
- Touch keyboard input with special key toolbar (Ctrl, Alt, Tab, Esc, arrows)
- Pinch-to-zoom font size
- Swipe between sessions
- Landscape mode with larger terminal area
- Copy/paste via long-press selection

**iOS-specific:**
- Smooth scrolling with iOS physics
- Keyboard accessories bar with special keys
- iPad: split-view and slide-over support
- iPad: external keyboard with full shortcut support (Cmd+C/V, etc.)
- 3D Touch / Haptic Touch for context menus

**Android-specific:**
- Custom key toolbar above system keyboard
- Tablet: multi-window / split-screen support
- Back gesture navigation
- Material You dynamic theming

**Data Flow:**
```
User types on keyboard
  -> controllerinput RPC (inputdata: bytes)
  -> agentmuxsrv-rs routes to PTY
  -> PTY output event fires
  -> eventsub delivers to mobile client
  -> xterm.dart terminal renders output
```

### 4. AI Agent View

Chat-style interface for interacting with AI agents.

**Features:**
- Message input with markdown preview
- Streaming response rendering (token-by-token)
- Tool approval prompts (approve/deny tool execution)
- Chat history (loaded via `getwaveaichat`)
- Model/preset selection
- Context attachment (paste text, reference files)

**Data Flow:**
```
User sends message
  -> aisendmessage RPC (streaming)
  -> streamwaveai delivers tokens
  -> UI renders incrementally
  -> Tool approval events -> user approves -> waveaitoolapprove
```

### 5. Settings

- Backend connection management
- Terminal preferences (font size, theme, scrollback)
- AI preferences (model, preset)
- App preferences (theme, haptics, notifications)
- Sync settings to/from backend via `setconfig` / `getfullconfig`

---

## Project Structure

```
agentmux-mobile/
├── lib/
│   ├── main.dart
│   ├── app.dart                      # App root, routing
│   ├── core/
│   │   ├── rpc/
│   │   │   ├── rpc_client.dart       # WebSocket RPC client
│   │   │   ├── rpc_types.dart        # Command/response types
│   │   │   └── event_bus.dart        # Event subscription manager
│   │   ├── models/
│   │   │   ├── block.dart            # Block object model
│   │   │   ├── tab.dart              # Tab object model
│   │   │   ├── workspace.dart        # Workspace object model
│   │   │   ├── connection.dart       # Backend connection config
│   │   │   └── settings.dart         # Settings type
│   │   └── providers/
│   │       ├── connection_provider.dart
│   │       ├── session_provider.dart
│   │       ├── settings_provider.dart
│   │       └── auth_provider.dart
│   ├── features/
│   │   ├── connections/
│   │   │   ├── connections_screen.dart
│   │   │   ├── add_connection_screen.dart
│   │   │   └── qr_scanner_screen.dart
│   │   ├── sessions/
│   │   │   ├── sessions_screen.dart
│   │   │   └── session_card.dart
│   │   ├── terminal/
│   │   │   ├── terminal_screen.dart
│   │   │   ├── terminal_keyboard.dart   # Special keys toolbar
│   │   │   └── terminal_provider.dart
│   │   ├── agent/
│   │   │   ├── agent_screen.dart
│   │   │   ├── message_bubble.dart
│   │   │   ├── tool_approval.dart
│   │   │   └── agent_provider.dart
│   │   └── settings/
│   │       └── settings_screen.dart
│   └── shared/
│       ├── widgets/                    # Reusable components
│       └── theme/                      # App theming
├── ios/                                # iOS runner (Xcode project)
├── android/                            # Android runner (Gradle project)
├── test/
├── pubspec.yaml
└── README.md
```

---

## RPC Client Design

The core of the app — a Dart WebSocket RPC client that mirrors the desktop's `WshRpcEngine`.

```dart
class AgentMuxRpcClient {
  final WebSocketChannel _channel;
  final Map<String, Completer<dynamic>> _pendingRequests = {};
  final Map<String, StreamController<dynamic>> _subscriptions = {};
  int _nextReqId = 1;

  /// Send an RPC command and await response
  Future<T> call<T>(String command, dynamic data) async {
    final reqId = 'req-${_nextReqId++}';
    final completer = Completer<T>();
    _pendingRequests[reqId] = completer;
    _channel.sink.add(jsonEncode({
      'command': command,
      'reqid': reqId,
      'data': data,
    }));
    return completer.future;
  }

  /// Subscribe to events (returns a stream)
  Stream<dynamic> subscribe(String eventType, {List<String>? scopes}) {
    // Send eventsub RPC, return stream that receives eventrecv messages
  }

  /// Send terminal input (fire-and-forget)
  void sendInput(String blockId, Uint8List data) {
    call('controllerinput', {
      'blockid': blockId,
      'inputdata': {'inputdata': base64Encode(data)},
    });
  }
}
```

---

## Platform Considerations

### Android

| Concern | Approach |
|---------|----------|
| Virtual keyboard | Custom key toolbar above system keyboard (Ctrl, Alt, Tab, Esc, arrows, pipe) |
| Screen size | Responsive layout — single pane on phone, split on tablet |
| Background | Keep WebSocket alive via foreground service when terminal active |
| Battery | Pause event subscriptions when app backgrounded |
| Haptics | Subtle haptic feedback on key presses |
| Min SDK | API 26 (Android 8.0) — covers 95%+ of devices |
| Distribution | Google Play Store + direct APK download |

### iOS

| Concern | Approach |
|---------|----------|
| Keyboard | Keyboard accessories bar with Ctrl, Alt, Tab, Esc, arrows |
| iPad | Split-view, slide-over, external keyboard with Cmd shortcuts |
| Background | iOS background modes — limited; use background URLSession for reconnect |
| Notifications | Push notifications via APNs for long-running command completion |
| Haptics | UIImpactFeedbackGenerator for key presses |
| Secure storage | iOS Keychain for auth tokens |
| Min version | iOS 16+ |
| Distribution | App Store (TestFlight for beta) |
| App Transport Security | TLS required by default — aligns with our WSS requirement |

---

## Security

- **Auth tokens** stored in platform secure storage (iOS Keychain / Android Keystore)
- **TLS** required for remote connections (WSS) — iOS ATS enforces this by default
- **Local connections** (same network) allowed over plain WS with explicit user opt-in
- **No credentials in logs** — authkey redacted from debug output
- **Certificate pinning** for known remote servers (optional)
- **Biometric lock** — optional Face ID / Touch ID (iOS) or fingerprint (Android) to open app

---

## Build & CI

```yaml
# pubspec.yaml (key dependencies)
dependencies:
  flutter:
    sdk: flutter
  flutter_riverpod: ^2.x
  xterm: ^4.x
  web_socket_channel: ^2.x
  go_router: ^14.x
  flutter_secure_storage: ^9.x
  mobile_scanner: ^5.x        # QR code scanning

dev_dependencies:
  flutter_test:
    sdk: flutter
  mocktail: ^1.x
  flutter_lints: ^4.x
```

**Build Commands:**
```bash
# Development
flutter run -d ios
flutter run -d android

# Release
flutter build ipa --release          # iOS (requires Xcode + signing)
flutter build appbundle --release    # Android (AAB for Play Store)
flutter build apk --release          # Android (APK for direct install)

# Tests
flutter test
```

**CI (GitHub Actions):**
- Android: Build on ubuntu runner, publish to Play Store via Fastlane
- iOS: Build on macOS runner, publish to TestFlight via Fastlane

---

## Phasing

### Phase 1 — Foundation
- [ ] Flutter project scaffold (Android + iOS)
- [ ] WebSocket RPC client (Dart port of WshRpcEngine)
- [ ] Connection manager (add, edit, delete, secure storage)
- [ ] Auth flow (manual authkey entry)
- [ ] Session browser (list workspaces, tabs, blocks)

### Phase 2 — Terminal
- [ ] Terminal view with xterm.dart
- [ ] Connect to existing PTY sessions (controllerresync)
- [ ] Keyboard input with special keys toolbar
- [ ] Create new terminal blocks
- [ ] Copy/paste support
- [ ] iPad external keyboard shortcuts

### Phase 3 — AI Agent
- [ ] Agent chat view with streaming responses
- [ ] Tool approval UI
- [ ] Chat history
- [ ] Model/preset selection
- [ ] Context attachment

### Phase 4 — Polish
- [ ] QR code pairing
- [ ] Settings sync with backend
- [ ] Push notifications for long-running commands (APNs + FCM)
- [ ] Background WebSocket keepalive (Android foreground service)
- [ ] Theming (match desktop AgentMux theme, Material You on Android)
- [ ] iPad split-view / slide-over support
- [ ] Biometric lock (Face ID / fingerprint)

### Phase 5 — Future
- [ ] mDNS auto-discovery of local AgentMux instances
- [ ] Direct SSH (dartssh2, no backend needed)
- [ ] Local Rust core via flutter_rust_bridge (offline config, local AI)
- [ ] Apple Watch companion (session status, quick commands)
- [ ] Widgets (iOS home screen, Android home screen)
- [ ] Siri Shortcuts / Android App Shortcuts for quick connect

---

## References

- [xterm.dart — Flutter terminal emulator](https://github.com/TerminalStudio/xterm.dart)
- [dartssh2 — Pure Dart SSH client](https://github.com/TerminalStudio/dartssh2)
- [flutter_rust_bridge](https://github.com/fzyzcjy/flutter_rust_bridge) (future, if local Rust needed)
- [Riverpod — State management](https://riverpod.dev/)
- [Flutter architecture recommendations](https://docs.flutter.dev/app-architecture/recommendations)
- [Flutter iOS building](https://docs.flutter.dev/platform-integration/ios)
- [Flutter Android building](https://docs.flutter.dev/platform-integration/android)
- [Linxr — SSH terminal in Flutter tutorial](https://dev.to/ai2th/linxr-part-3-ssh-terminal-in-flutter-36oe)
