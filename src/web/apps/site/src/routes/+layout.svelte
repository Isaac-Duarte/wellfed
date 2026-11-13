<script lang="ts">
  import "@wellfed/ui/base.css";
  import "#lib/site.css";
  import { resolve } from "$app/paths";
  import type { Snippet } from "svelte";
  import {
    DISCORD,
    DiscordMark as Discord,
    GitHubMark as GitHub,
    HOME,
    REPO,
    favicon,
    lockup,
    lockupOnLight,
    themeColor,
  } from "@wellfed/ui";
  import { DOCS } from "#lib/links.ts";

  let { children }: { children: Snippet } = $props();

  const title = "Wellfed: open firmware for your pet feeder";
  const description =
    "Open firmware that runs the PETLIBRO Granary PLAF103 pet feeder on your own Wi-Fi. No cloud, no account.";
</script>

<svelte:head>
  <title>{title}</title>
  <meta name="description" content={description} />
  <link rel="canonical" href="{HOME}/" />
  <link rel="icon" href={favicon} type="image/svg+xml" />
  <meta name="theme-color" content={themeColor.light} media="(prefers-color-scheme: light)" />
  <meta name="theme-color" content={themeColor.dark} media="(prefers-color-scheme: dark)" />
  <meta property="og:type" content="website" />
  <meta property="og:site_name" content="Wellfed" />
  <meta property="og:title" content={title} />
  <meta property="og:description" content={description} />
  <meta property="og:url" content="{HOME}/" />
  <meta property="og:image" content="{HOME}/og.png" />
  <meta property="og:image:width" content="1200" />
  <meta property="og:image:height" content="630" />
  <meta property="og:image:alt" content="The Wellfed logo and the line: open firmware for your pet feeder" />
  <meta name="twitter:card" content="summary_large_image" />
</svelte:head>

<a class="skip" href="#main">Skip to content</a>

<header>
  <div class="wrap bar">
    <a href={resolve("/")} class="home">
      <picture>
        <source srcset={lockup} media="(prefers-color-scheme: dark)" />
        <img src={lockupOnLight} alt="Wellfed" width="152" height="35" />
      </picture>
    </a>
    <nav aria-label="Primary">
      <a href={DISCORD} rel="external" class="discord" aria-label="Discord"><Discord /></a>
      <a href={REPO} rel="external" class="github"><GitHub /><span>GitHub</span></a>
    </nav>
  </div>
</header>

<main id="main">
  {@render children()}
</main>

<footer>
  <div class="wrap grid">
    <div class="notes">
      <p>
        <strong>Independent community project.</strong> Not affiliated with or endorsed by PETLIBRO. PETLIBRO and Granary
        are trademarks of their owners, named here only to say which hardware the firmware runs on.
      </p>
      <p>
        <strong>Experimental.</strong> Keep another way to feed your pet while you try it.
      </p>
    </div>
    <div class="meta">
      <p>
        Code under the <a href={DOCS.license} rel="external">MIT license</a>; the logo and brand under
        <a href={DOCS.assets} rel="external">CC BY 4.0</a>.
      </p>
    </div>
  </div>
</footer>

<style>
  .bar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-4);
    min-height: 72px;
  }
  .home {
    display: flex;
    border-radius: var(--radius-control);
  }
  .home img {
    display: block;
    height: 26px;
    width: auto;
  }
  nav {
    display: flex;
    align-items: center;
    gap: var(--space-5);
    font-size: var(--text-base);
    font-weight: var(--weight-medium);
  }
  nav a {
    color: var(--color-text-muted);
    text-decoration: none;
    border-radius: var(--radius-control);
  }
  nav a:hover {
    color: var(--color-text);
  }
  .discord {
    display: inline-flex;
    align-items: center;
  }
  .github {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    color: var(--color-text);
  }

  footer {
    padding-block: var(--space-7) 64px;
    border-top: 1px solid var(--color-line);
    color: var(--color-text-muted);
    font-size: var(--text-sm);
  }
  .grid {
    display: grid;
    grid-template-columns: 3fr 2fr;
    gap: var(--space-6);
  }
  @media (max-width: 760px) {
    .grid {
      grid-template-columns: 1fr;
    }
  }
  footer p {
    margin: 0 0 var(--space-3);
    max-width: 60ch;
  }
  footer strong {
    color: var(--color-text);
    font-weight: var(--weight-semibold);
  }
</style>
