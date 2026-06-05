---
type: fix
scope: linux
title: Bundle CEF 148 Vulkan SwiftShader runtime files into AppImage
---
CEF 148 ships its software-rasterizer fallback as Vulkan SwiftShader
(`libvk_swiftshader.so` + `vk_swiftshader_icd.json` + `libvulkan.so.1`),
replacing the GLES-based SwiftShader path that CEF 146 used. The current
bundle scripts don't copy these — Chromium's GPU process fails to find
the Vulkan ICD and the renderer process crashes with SIGTRAP at GPU
init, leaving the AppImage on a recovery screen.

Also adds `headless_command_resources.pak`, a new resource bundle in
CEF 148.

Verified end-to-end:
  task build:host && task build:backend && task build:frontend && task bundle
  bash scripts/build-appimage-linux.sh /tmp
  /tmp/AgentMux_0.42.0_amd64.AppImage
  → 8 procs alive, libcef.so loaded (Chrome/148.0.7778.180), backend
    handling RPC, renderer up, no SIGTRAP. The pre-fix AppImage from
    yesterday SIGTRAP'd at gpu init.
