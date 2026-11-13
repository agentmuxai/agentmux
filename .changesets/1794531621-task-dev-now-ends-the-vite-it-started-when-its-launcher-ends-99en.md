---
type: patch
---

task dev now ends the Vite it started when its launcher ends (normal quit, failed start, signal), on Linux, macOS and Windows; scripts/dev-vite.sh records the Vite's PID and port, and a Vite it did not start is never touched
