---
type: patch
---

build: the frontend typechecks with TypeScript 7 (tsc 7.0.2); the tools that read the compiler API (typescript-eslint, knip, organize-imports, the gen scripts) stay on the TypeScript 6 API package, and tsconfig.json drops the options TypeScript 7 removed (baseUrl, downlevelIteration)
