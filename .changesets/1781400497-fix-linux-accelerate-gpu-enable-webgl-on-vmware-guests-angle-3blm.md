---
type: patch
---

fix(linux): capability-probed ANGLE backend precedence (hardware Vulkan → hardware GL → SwiftShader) — fixes burst-paint terminals and enables hardware WebGL on VMware/SVGA3D (and any no-Vulkan-but-has-GL) guests, with no vendor gate
