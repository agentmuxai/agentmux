// @ts-check

import eslint from "@eslint/js";
import eslintConfigPrettier from "eslint-config-prettier";
import tseslint from "typescript-eslint";

const baseConfig = tseslint.config(eslint.configs.recommended, ...tseslint.configs.recommended);

export default [baseConfig, eslintConfigPrettier];
