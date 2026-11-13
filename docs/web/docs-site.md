# docs.well-fed.dev

This page is for maintainers of the docs site, which serves people who own or might run a feeder, homelabbers first. It publishes the [owner guide](../guide/README.md) and a few reference pages, rendered by the SvelteKit app in `src/web/apps/docs` and served as static files by a Cloudflare Worker. Contributor and hardware pages stay on GitHub, and the site links to them there.

`nav.ts` is the sidebar and the only list of what is published; an `onGitHub` entry puts a GitHub-only doc in the sidebar as a link there (the development guide, today). The Markdown stays plain GitHub-flavored Markdown that reads the same on GitHub: the site reads it at build time, never changes it, and the [writing guide](../development/writing.md) applies to all of it.

## What it is

| Area | Choice |
| --- | --- |
| Build | SvelteKit 3 with `adapter-static`; every page `nav.ts` lists is prerendered from `docs/` at build time |
| Routes | `docs/guide/` is the site's root, so `docs/guide/install.md` is `/install`. Other published docs keep their path. Two files on one route fail the build |
| Markdown | unified (remark, GFM, rehype) in `src/lib/server/content.ts`, not mdsvex: the docs are never compiled as Svelte, so a `{` or `<` in prose is just text |
| Links | A link to a published doc becomes its route; any other link into the repository becomes that file on GitHub. The prerender crawl fails the build on a link or `#anchor` on the site that doesn't resolve, or a link to a doc that doesn't exist. `cargo xtask check-repository` checks anchors into GitHub-only pages |
| Code | Shiki at build time, one theme whose colors are the token roles, so code follows light and dark |
| Diagrams | A ```` ```mermaid ```` block naming an SVG (`%% svg: name.svg`) becomes that SVG, inlined so `docs.css` colors its `.d-` classes from the tokens, while GitHub draws the Mermaid. `cargo xtask check-repository` holds each pair to its hashes in `docs/diagrams.lock` ([writing guide](../development/writing.md#diagrams)) |
| Markdown copy | Each page's source is prerendered beside it as `<page>.md` (the root as `/index.md`), with every link and image made absolute and the tabs markers dropped. The "Copy as Markdown" button copies that file |
| App \| CLI tabs | `<!-- tabs -->` and `<!-- /tabs -->` around `### App` and `### CLI` become a switch ([writing guide](../development/writing.md#app-and-cli-steps)). The choice holds on every page, kept in `localStorage` and set before first paint by one inline script (`tabs.ts`), whose hash `vite.config.ts` adds to the CSP. Without JavaScript both sections show; a malformed set fails the build |
| Test log verdicts | A verdict opening an entry in the test log (`guide/how-it-works/testing.md`) becomes a colored label ([writing guide](../development/writing.md#test-log-verdicts)). An entry with no verdict, two verdicts or a phrase outside the set fails the build |
| Images | A PNG beside a page is served beside its route, sized from the file's header; only images a published page shows are published. A missing file, missing alt text or any other format fails the build |
| Search | Pagefind indexes the published pages in `build/` after the prerender and serves its index from `/pagefind/`, so a query never leaves the site |
| Style | The [tokens](../../assets/brand/tokens.json) and `@wellfed/ui`'s base, as on [well-fed.dev](site.md): type and hairlines, no cards or gradients. Light or dark follows the system |
| Requests | None off its own origin. Each page carries a CSP `<meta>` with a hash for every inline script, and `static/_headers` sets the rest, allowing one inline style by hash: SvelteKit's route announcer. The e2e tests serve `_headers`, so a violation fails them |
| Tests | `svelte-check`, ESLint and Prettier with the workspace; Vitest on the published list, routes, links, tabs, verdicts, images and diagrams, and that every Mermaid block under `docs/` parses (Mermaid in jsdom); and Playwright against the built site at desktop and phone width |

## Adding a page

Most new docs stay on GitHub with no extra step. A page belongs on the site only if an owner needs it, and those go under `docs/guide/`.

1. Write the page and link it from its folder's README, as usual.
2. To publish it, add it to `src/web/apps/docs/src/lib/nav.ts` in its group. Give it a `label` if its heading is long.
3. Run the build (below). It fails on a broken link or anchor, and the nav test fails if a page under `docs/guide/` is missing from the sidebar. If Vitest says a new doc doesn't exist, delete its cache folder (`.vitest-cache`, under the workspace's `node_modules`), since it doesn't notice new files.

## Develop

```sh
pnpm -C src/web install
pnpm -C src/web generate
pnpm -C src/web --filter @wellfed/docs dev      # http://localhost:5173, without search
pnpm -C src/web --filter @wellfed/docs build    # writes src/web/apps/docs/build/, with the search index
pnpm -C src/web --filter @wellfed/docs e2e
```

The dev server reloads when a doc changes. Search needs the index, so it works only in a build: `pnpm -C src/web --filter @wellfed/docs preview` serves `build/` the way the Worker does, from `src/web/scripts/serve.ts` (`vite preview` would miss the index and `_headers`). `make check-web` builds, checks and tests the site with the other app.

## Deploy

The settings mirror [well-fed.dev's](site.md#deploy): Cloudflare Workers Builds builds from the repository with its Git integration, nothing in the repository holds a Cloudflare credential, and the Worker has no script ([`wrangler.jsonc`](../../src/web/apps/docs/wrangler.jsonc) serves the build as static assets).

| Setting | Value |
| --- | --- |
| Worker name | `wellfed-docs` (must match `name` in `wrangler.jsonc`) |
| Production branch | `master` |
| Build command | `corepack enable && pnpm -C src/web install --frozen-lockfile && pnpm -C src/web generate && pnpm -C src/web --filter @wellfed/docs build` |
| Deploy command | `npx wrangler@4.152.0 deploy --config src/web/apps/docs/wrangler.jsonc` |
| Non-production branch deploy command | `npx wrangler@4.152.0 versions upload --config src/web/apps/docs/wrangler.jsonc` |
| Environment | `NODE_VERSION=22.18.0` |
| Custom domain | `docs.well-fed.dev` |
