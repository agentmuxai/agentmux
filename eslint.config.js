// @ts-check

import eslint from "@eslint/js";
import eslintConfigPrettier from "eslint-config-prettier";
import tseslint from "typescript-eslint";

const baseConfig = tseslint.config(eslint.configs.recommended, ...tseslint.configs.recommended);

// `tseslint.config(...)` already returns a flat-config ARRAY; nesting it
// inside another array (the old `export default [baseConfig, ...]`) gave
// ESLint 8's flat-config loader a config array containing a config array,
// which it rejects outright ("TypeError: Unexpected array") -- eslint has
// never actually run successfully against this repo. Spreading it here is
// the fix.
export default [...baseConfig, eslintConfigPrettier];
