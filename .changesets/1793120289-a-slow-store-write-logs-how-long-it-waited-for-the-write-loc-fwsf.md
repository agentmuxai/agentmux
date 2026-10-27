---
type: patch
---

A slow store write logs how long it waited for the write lock, ran, and committed, so a long hold of the store connection says whether it waited on another srv or on the disk
