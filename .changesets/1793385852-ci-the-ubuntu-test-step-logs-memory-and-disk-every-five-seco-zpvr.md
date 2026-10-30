---
type: patch
---
CI: the ubuntu test leg no longer gets shut down mid-build: it builds the srv unit test binary before the srv binary so two 8 GB compiles never overlap, has 12 GB more swap, and logs memory and disk every five seconds
