---
type: patch
---

release and install: each release carries a second signature of its envelope in SSH signature form, made by the same key, so a stock Mac's ssh-keygen can check the release when its openssl cannot; install.sh uses it and refuses a release from the first one that carries it that lacks it
