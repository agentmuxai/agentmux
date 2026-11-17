---
type: patch
---

An Anthropic API key can now be used by container agents: the container never receives the key, only the gateway's address and a token for that agent and account that works from that container alone (on Linux; on macOS and Windows from this machine). Agent containers no longer have the raw-socket capability, so tools such as ping may stop working in them, and a container made before this is recreated once.
