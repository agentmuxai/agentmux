---
type: patch
---

fix(lan): mDNS recovery keeps retrying with a backoff instead of giving up after three tries, logs the OS error and who holds UDP 5353 on each attempt, and mdns-sd's own log lines now reach the srv log (the log-to-tracing bridge was never installed)
