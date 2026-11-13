# TypeScript standard

The rules for the web frontends in `src/web/` (TypeScript and Svelte), the landing page and the docs site, why each exists, and what enforces it; every rule marked with a tool runs in `make check-web`. [AGENTS.md](../../AGENTS.md#web-conventions) keeps the short form. The server and the bench have no web UI: owners use the [app and the CLI](../firmware/product-architecture.md#frontends), and the bench has its [command line and agent tools](../devices/plaf103/bench-server.md).

## Layout

```text
src/web/
  apps/site/           the project's landing page, prerendered to static files (see docs/web/site.md)
  apps/docs/           the docs/ folder as a searchable site (see docs/web/docs-site.md)
  packages/tokens/     CSS generated from assets/brand/tokens.json, and the contrast check
  packages/ui/         the page base and the components both sites share, over the tokens
  scripts/             workspace checks run by Node (the license check)
  eslint.config.js     the lint rules below
```

The tokens come from one file, `assets/brand/tokens.json`, which the Flutter theme reads too, so the app and the web can't drift.

## Rules

| Rule | Why | Enforced by |
| --- | --- | --- |
| **Strict types.** `strict`, `noUncheckedIndexedAccess`, `noImplicitOverride` and `verbatimModuleSyntax` everywhere. | Index reads and optional fields are handled where they happen. | `tsconfig.base.json`; `tsc` and `svelte-check --fail-on-warnings` |
| **No `any`, no `!`, no `as`.** Narrow with a check (`instanceof`, `in`, a type guard) instead of asserting; `as const` is fine. | An assertion moves a wrong assumption from where it is made to where it crashes. | ESLint (`no-explicit-any`, `no-non-null-assertion`, `consistent-type-assertions: never`) |
| **The API layer is the boundary.** An app's `src/lib/api` is where a response body becomes a type, and the only place assertions are allowed. | One place to audit what a response is trusted to hold. | ESLint override for `apps/*/src/lib/api/**` |
| **Generated contracts.** Colors, spacing and type arrive through `@wellfed/tokens`; never hand-write a hex value. Generated files are rewritten, never edited, and committed with the change that caused them. | Two copies drift. | `tokens.test.ts` fails when `tokens.gen.css` is stale; ESLint and Prettier skip `*.gen.*` |
| **Only the API layer touches the network.** No `fetch`, `EventSource`, `BroadcastChannel`, `WebSocket` or `XMLHttpRequest` elsewhere. | Retry rules live in one place. | ESLint (`no-restricted-globals`) |
| **Runes only.** `$state`, `$derived` and `$props`; no `svelte/store`. Prefer `$derived` to `$effect`, which only syncs with something outside Svelte, and says why. | One reactivity model. | ESLint for stores; review for effects |
| **Zero warnings.** typescript-eslint `strict-type-checked` and `stylistic-type-checked`, eslint-plugin-svelte, `--max-warnings 0`. | A warning nobody fixes hides the next one. | `pnpm lint` |
| **Deliberate exceptions.** A disable covers one line and gives its reason after `--`, as `#[expect(…, reason)]` does in Rust. | The next reader knows whether it still applies. | ESLint (`eslint-comments/require-description`, `no-unused-disable`) |
| **One format.** Prettier with the Svelte plugin, 120 columns. | No formatting in review. | `pnpm format:check` |
| **Comments explain why:** an invariant, a browser quirk, a feeding-safety rule. | As in Rust. | Review |
| **Small components.** Typed props, snippets for content, one concern each; `packages/ui` components never fetch. | A shared widget stays reusable and testable. | Review |
| **Accessible.** WCAG 2.2 AA: semantic HTML first, every control usable by keyboard, visible focus, `prefers-reduced-motion` respected, and tests that find elements by role and name. | People use these pages with a keyboard and a screen reader too. | The contrast check for tokens; review for the rest |

## Dependencies

Set in [pnpm-workspace.yaml](../../src/web/pnpm-workspace.yaml) and checked on every install and in `make check-web`:

- **Exact versions and a frozen lockfile.** `savePrefix: ""` pins what `pnpm add` writes; every check installs with `--frozen-lockfile`.
- **A week before install.** `minimumReleaseAge` is 7 days, so a compromised release is usually pulled before it reaches the lockfile, and pnpm re-verifies the lockfile against it on install. An exception names the exact version and when it can be dropped; `cargo xtask check-repository` refuses a bare name or a pattern, which would exempt every release to come.
- **No install scripts** unless a package is listed under `allowBuilds` (none today), with `strictDepBuilds` failing the install otherwise.
- **Licenses.** `pnpm run licenses` ([scripts/licenses.ts](../../src/web/scripts/licenses.ts)) checks every installed package, dev tools included, against the same permissive set as [deny.toml](../../deny.toml), plus 0BSD, BlueOak, CC-BY-4.0 (data) and OFL-1.1 (the bundled fonts).
- **Justify each package** in the change that adds it: what it does that the platform or an existing package doesn't.

## Commands

From `src/web/`, with Node 22.18 or later (it runs the TypeScript scripts directly) and pnpm 11:

| Command | Does |
| --- | --- |
| `pnpm install --frozen-lockfile` | Install exactly the lockfile |
| `pnpm generate` | Rewrite `tokens.gen.css` from `assets/brand/tokens.json`; fails if a named pair misses its contrast minimum |
| `pnpm check` | ESLint, Prettier, licenses, `tsc` for the scripts, then each package's `svelte-check` or `tsc` |
| `pnpm test` | Vitest in each package (the contrast and drift tests included) |
| `pnpm format` | Rewrite formatting |
| `make check-web` (repository root) | Install, build, all of the above, and Playwright of both sites. It runs no Rust |
