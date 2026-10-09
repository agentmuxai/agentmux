---
type: patch
---

Requests to the backend get their auth header from one helper, which sends none when the app's host supplies no key, so a proxy in front of the backend can add it.
