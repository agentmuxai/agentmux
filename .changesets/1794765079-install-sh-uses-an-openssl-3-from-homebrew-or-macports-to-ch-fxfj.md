---
type: patch
---

install.sh: uses an OpenSSL 3 from Homebrew or MacPorts to check the release signature when the system openssl cannot, and says plainly which check ran; refuses the macOS Intel server up front; says to install ca-certificates when curl cannot check certificates; prints the full path and the PATH line after installing into ~/.local/bin
