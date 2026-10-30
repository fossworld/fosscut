import js from "@eslint/js";
import reactHooks from "eslint-plugin-react-hooks";
import globals from "globals";
import tseslint from "typescript-eslint";

export default tseslint.config(
  { ignores: ["dist"] },
  js.configs.recommended,
  ...tseslint.configs.recommended,
  reactHooks.configs.flat["recommended-latest"],
  {
    languageOptions: { globals: globals.browser },
    rules: {
      // Components never reach Tauri directly: only src/engine may (ADR-0010, item 4).
      "no-restricted-imports": [
        "error",
        { patterns: [{ group: ["@tauri-apps/*"], message: "Use the engine client in src/engine." }] },
      ],
    },
  },
  {
    files: ["src/engine/**"],
    rules: { "no-restricted-imports": "off" },
  },
);
