// Every outbound link the docs site adds to @wellfed/ui's, so moving the site is one edit.
import { HOME, REPO } from "@wellfed/ui";

export const SITE = "https://docs.well-fed.dev";

/** A file or folder in the repository on GitHub, from its path at the root. */
export const blob = (path: string) => `${REPO}/blob/master/${path}`;
export const edit = (path: string) => `${REPO}/edit/master/${path}`;

/** Where a page's Markdown is served: `/devices/plaf103.md`, the root as `/index.md`. */
export const markdownPath = (slug: string) => `/${slug === "" ? "index" : slug}.md`;

/** The social card well-fed.dev already serves, rendered from assets/brand/og.svg. */
export const OG_IMAGE = `${HOME}/og.png`;
