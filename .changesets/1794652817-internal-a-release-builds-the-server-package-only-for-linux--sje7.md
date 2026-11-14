---
type: patch
---

Internal: a release builds the server package only for Linux x64, Linux arm64 and macOS arm64, so it never waits on a scarce runner; the Linux build runs under the memory guard, the no-certificates check pulls from Amazon's public mirror by digest, and the programs carry a build time stamp
