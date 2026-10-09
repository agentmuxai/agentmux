# SPEC: Build the frontend for a host other than the desktop app

**Date:** 2026-10-09
**Status:** implemented (this spec's PR)
**Author:** Agent4
**Builds on:** `SPEC_HOST_API_SEAM_2026_09_26.md` (the UI reaches its host only through `AppApi`, and asks `HostCaps` what it can do).

## 1. Problem

The host seam made the UI host-independent, but its entry wasn't: `frontend/bootstrap.ts` imported the desktop (CEF) host's startup directly. It installed the CEF log pipe and error forwarder, sent CEF's first-paint signal, and called `setupCefApi()`. A different host (a test harness, a browser host for a headless srv, an embedding) could only be built by editing `bootstrap.ts`.

## 2. Design

**A host module** (`frontend/app/host/host-module.ts`) is the host's startup, as three hooks:

| Hook | When | Desktop host |
|---|---|---|
| `early()` | first, before any other startup code | starts the log pipe and the error forwarder (both send to the host's log file) |
| `firstPaint(label)` | two animation frames after load | `report_first_paint`, which gates the native window's first show on Linux |
| `setup()` | before the app starts; `getApi()` works once it resolves | `setupCefApi()` |

`bootstrap.ts` imports the build's module as `@host-module` and calls the hooks. It no longer imports anything of the desktop host's.

**The desktop host is the default:** `tsconfig.json` maps `@host-module` to `frontend/cef-host-module.ts`. A build that sets nothing gets exactly the bundle it got before.

**Another host is chosen at build time:**

```sh
AGENTMUX_HOST_MODULE=/path/to/host.ts \
AGENTMUX_HOST_TSCONFIG=/path/to/tsconfig.json \
vite build --outDir /path/to/dist
```

- `AGENTMUX_HOST_MODULE`: a module exporting `hostModule: HostModule`. `vite.config.ts` aliases `@host-module` to it.
- `AGENTMUX_HOST_TSCONFIG` (optional): a tsconfig that includes the module's files, so they can import the UI's `@/` paths. It can extend this repo's `tsconfig.json`; its `include` must also list this repo's `frontend/**/*`.
- The dev server allows the module's directory, since it serves files under this repo only.

The module typically also builds its `AppApi` (with its own `HostCaps`, `app/host/host-caps.ts`) and installs it as `window.api` in `setup()`.

**Not in scope:** a separate entry page. `index.html` stays the entry; a host that wants a different document title or startup look sets it from its module, or after the build.

## 3. Performance

None at runtime. The module is chosen when the bundle is built; the desktop bundle contains the same code as before, and the hooks are three direct calls.

## 4. Testing

- `host-module-build.test.ts`: with nothing set, `@host-module` is left to `tsconfig.json`; with `AGENTMUX_HOST_MODULE` set, it is that module's absolute path.
- `host-boundary.test.ts`: `bootstrap.ts` leaves the seam and `cef-host-module.ts` joins it.
- Checked by hand: a build with an outside module contains that module and none of the desktop host's startup (no `report_first_paint`, no log pipe), and a default build still contains both.
