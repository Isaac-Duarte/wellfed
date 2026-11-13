// ESLint for every web package. The rules and their reasons are in
// docs/development/typescript.md; this file is how they are enforced.
import js from "@eslint/js";
import comments from "@eslint-community/eslint-plugin-eslint-comments/configs";
import prettier from "eslint-config-prettier";
import svelte from "eslint-plugin-svelte";
import globals from "globals";
import ts from "typescript-eslint";

// The network is reached only through an app's lib/api, so every request a
// page makes (the docs site's Markdown and search index) is in one place,
// where the CSP's connect-src can be read against it.
const network = ["fetch", "EventSource", "BroadcastChannel", "WebSocket", "XMLHttpRequest"].map((name) => ({
  name,
  message: "Only an app's lib/api reaches the network.",
}));

const stores = {
  patterns: [{ group: ["svelte/store"], message: "Use runes ($state, $derived) instead of stores." }],
};

export default ts.config(
  {
    ignores: [
      "**/dist/",
      "**/build/",
      "**/.svelte-kit/",
      "**/test-results/",
      "**/playwright-report/",
      "**/.vite/",
      "**/*.gen.css",
      "**/*.gen.ts",
    ],
  },

  js.configs.recommended,
  ts.configs.strictTypeChecked,
  ts.configs.stylisticTypeChecked,
  svelte.configs.recommended,
  comments.recommended,
  prettier,
  svelte.configs.prettier,

  {
    languageOptions: {
      globals: { ...globals.browser },
      parserOptions: {
        projectService: true,
        tsconfigRootDir: import.meta.dirname,
        extraFileExtensions: [".svelte"],
      },
    },
    rules: {
      "@typescript-eslint/no-explicit-any": "error",
      "@typescript-eslint/no-non-null-assertion": "error",
      // No `as` outside generated code: narrow with a check instead, so a
      // wrong assumption fails where it is made.
      "@typescript-eslint/consistent-type-assertions": ["error", { assertionStyle: "never" }],
      "@typescript-eslint/restrict-template-expressions": ["error", { allowNumber: true }],
      "@eslint-community/eslint-comments/require-description": ["error", { ignore: [] }],
      "@eslint-community/eslint-comments/no-unused-disable": "error",
      "no-restricted-globals": ["error", ...network],
      "no-restricted-imports": ["error", stores],
      // `x != null` is the one loose comparison: it means "neither null nor undefined".
      eqeqeq: ["error", "always", { null: "ignore" }],
    },
  },

  {
    files: ["**/*.svelte", "**/*.svelte.ts"],
    languageOptions: {
      parserOptions: { parser: ts.parser, svelteConfig: undefined },
    },
  },

  // The API layer is the one place that reaches the network, and the boundary
  // where a response body becomes one of the generated types. Generated files
  // (*.gen.ts) are not linted at all: they are rewritten, never edited.
  {
    files: ["apps/*/src/lib/api/**"],
    rules: {
      "no-restricted-globals": "off",
      "@typescript-eslint/consistent-type-assertions": "off",
    },
  },

  // Node scripts: the build-time generators and the e2e server.
  {
    files: ["**/*.mjs", "**/*.config.js", "**/*.config.ts", "scripts/**", "packages/tokens/src/generate.ts"],
    languageOptions: { globals: { ...globals.node } },
  },
  {
    files: ["**/*.js", "**/*.mjs"],
    extends: [ts.configs.disableTypeChecked],
  },
);
