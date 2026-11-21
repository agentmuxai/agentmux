---
type: patch
---

Windows builds: the Taskfile's backend, host, bundle and dev steps read cargo's output from CARGO_TARGET_DIR when it is set (a target folder on another drive works), and a CI gate keeps a literal target/release out of the Taskfile
