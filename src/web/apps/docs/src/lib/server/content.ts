// The docs nav.ts lists, rendered to HTML at build time. The Markdown stays
// plain GitHub-flavored Markdown that reads the same on GitHub; this module is
// the one place that turns it into site pages: relative links become site
// routes or GitHub URLs, code is highlighted, and each page's headings feed
// its table of contents. It runs only during prerender, so none of it ships.
// A doc nav.ts doesn't list stays on GitHub and is never rendered or searched.
import { posix } from "node:path";
import type { Element, Root as Hast } from "hast";
import { toString } from "hast-util-to-string";
import type { Root as Mdast, Paragraph, PhrasingContent, RootContent, Strong } from "mdast";
import rehypeSlug from "rehype-slug";
import rehypeStringify from "rehype-stringify";
import remarkGfm from "remark-gfm";
import remarkParse from "remark-parse";
import remarkRehype from "remark-rehype";
import { createCssVariablesTheme, createHighlighter, type ShikiTransformer } from "shiki";
import { unified, type Plugin } from "unified";
import { SKIP, visit } from "unist-util-visit";
import type { Doc, Heading } from "#lib/doc.ts";
import { SITE, blob, edit } from "#lib/links.ts";
import { PUBLISHED } from "#lib/nav.ts";

const sources = import.meta.glob<string>("../../../../../../../docs/**/*.md", {
  query: "?raw",
  import: "default",
  eager: true,
});

const drawings = import.meta.glob<string>("../../../../../../../docs/**/*.svg", {
  query: "?raw",
  import: "default",
  eager: true,
});

// Inlined so the bytes reach the prerender without a hashed asset URL: a PNG
// keeps a stable address beside its page, which a copied `.md` can link to.
const pictures = import.meta.glob<string>("../../../../../../../docs/**/*.png", {
  query: "?inline",
  import: "default",
  eager: true,
});

const underDocs = (key: string) => key.replace(/^(\.\.\/)+docs\//, "");
const files = new Map(Object.entries(sources).map(([key, text]) => [underDocs(key), text]));
const svgs = new Map(Object.entries(drawings).map(([key, text]) => [underDocs(key), text]));
const pngs = new Map(Object.entries(pictures).map(([key, dataUrl]) => [underDocs(key), dataUrl]));

const published = new Set(PUBLISHED);

/** A path under docs/ with the guide's folder taken off: the guide is the site's root. */
const atSiteRoot = (path: string) => path.replace(/^guide\//, "");

/** `guide/install.md` → `install`, `firmware/README.md` → `firmware`, `guide/README.md` → the root. */
export function routeOf(file: string): string {
  return atSiteRoot(file)
    .replace(/\.md$/, "")
    .replace(/(^|\/)README$/, "");
}

/** Every path under docs/ that holds Markdown, published or not. */
export const sourceFiles: readonly string[] = [...files.keys()];

export const sourceOf = (file: string) => files.get(file);

/**
 * A link as written in docs/ is relative to the file and works on GitHub.
 * On the site, a link to a published doc becomes its route (its #anchor kept,
 * and checked by the prerender crawl). A link to any other doc, or anywhere
 * else in the repository, becomes that file on GitHub. A doc that doesn't exist
 * fails the build here, since the crawl can't see a link that leaves the site.
 */
export function resolveLink(url: string, file: string): string {
  if (url === "" || url.startsWith("#") || /^[a-z][a-z\d+.-]*:/i.test(url)) return url;
  // GitHub reads `/src/x.rs` from the repository's root and the site from its
  // own, so no one rewrite serves both: write it relative to the page.
  if (url.startsWith("/")) throw new Error(`docs/${file}: link "${url}" must be relative to the page`);

  const [target = "", ...rest] = url.split("#");
  const hash = rest.length > 0 ? `#${rest.join("#")}` : "";
  const path = posix.normalize(posix.join("docs", posix.dirname(file), target));

  if (path.startsWith("docs/")) {
    const inDocs = path.slice("docs/".length);
    const folderReadme = posix.join(inDocs, "README.md");
    const doc = inDocs.endsWith(".md") ? inDocs : files.has(folderReadme) ? folderReadme : undefined;
    if (doc !== undefined) {
      if (!files.has(doc)) throw new Error(`docs/${file}: links to docs/${doc}, which doesn't exist`);
      return published.has(doc) ? `/${routeOf(doc)}${hash}` : `${blob(`docs/${doc}`)}${hash}`;
    }
  }
  return `${blob(path.replace(/\/$/, ""))}${hash}`;
}

// A title can repeat its URL, and portableMarkdown finds a URL by its text in
// the source, so a titled link could be rewritten in the wrong place. GitHub
// shows a title only as a hover, so the docs go without.
function refuseTitle(node: { url: string; title?: string | null }, file: string) {
  if (node.title != null) throw new Error(`docs/${file}: link "${node.url}" has a title; drop it`);
}

const rewriteLinks: Plugin<[string], Mdast> = (file) => (tree) => {
  visit(tree, (node) => {
    if (node.type !== "link" && node.type !== "definition") return;
    refuseTitle(node, file);
    node.url = resolveLink(node.url, file);
  });
};

// GitHub's alert syntax, `> [!WARNING]` as a blockquote's first line, becomes
// an <aside> with its kind as a title, as GitHub renders it.
const ALERTS = { NOTE: "Note", TIP: "Tip", IMPORTANT: "Important", WARNING: "Warning", CAUTION: "Caution" };
const ALERT = /^\[!(NOTE|TIP|IMPORTANT|WARNING|CAUTION)\]\s*/;

function isAlertKind(kind: string): kind is keyof typeof ALERTS {
  return kind in ALERTS;
}

const alerts: Plugin<[], Mdast> = () => (tree) => {
  visit(tree, "blockquote", (node) => {
    const [first] = node.children;
    if (first?.type !== "paragraph") return;
    const [lead] = first.children;
    if (lead?.type !== "text") return;
    const kind = ALERT.exec(lead.value)?.[1];
    if (kind === undefined || !isAlertKind(kind)) return;

    lead.value = lead.value.replace(ALERT, "");
    if (lead.value === "") first.children.shift();
    if (first.children.length === 0) node.children.shift();
    node.data = { hName: "aside", hProperties: { className: ["alert", `alert-${kind.toLowerCase()}`] } };
    node.children.unshift({
      type: "paragraph",
      data: { hProperties: { className: ["alert-title"] } },
      children: [{ type: "text", value: ALERTS[kind] }],
    });
  });
};

// The test log's verdicts: GitHub shows plain bold text, and the site shows a
// label whose color says how much is still wrong. The set is closed and every
// entry needs exactly one, so a page can't slip in a softer phrase or leave a
// result out. Only the listed pages are read, since "**Open.**" could open a
// paragraph anywhere else with no such meaning.
const VERDICT_PAGES: ReadonlySet<string> = new Set(["guide/how-it-works/testing.md"]);
const VERDICTS: ReadonlyMap<string, string> = new Map([
  ["Passed", "passed"],
  ["Passed with gaps", "gaps"],
  ["Fixed", "fixed"],
  ["Partly fixed", "partly"],
  ["Open", "open"],
  ["Measured", "measured"],
]);

/** The text of bold that holds nothing else, such as a cell's `**Open**`. */
function boldText(node: PhrasingContent | undefined): string | undefined {
  if (node?.type !== "strong") return undefined;
  const [only, extra] = node.children;
  return only?.type === "text" && extra === undefined ? only.value : undefined;
}

function markVerdict(bold: Strong, name: string): void {
  bold.data = { hProperties: { className: ["verdict", `verdict-${VERDICTS.get(name) ?? ""}`] } };
  bold.children = [{ type: "text", value: name }];
}

// A verdict's own paragraph, so the label sits above its findings and a screen
// reader or a search excerpt pauses after it, with no full stop left to show.
function splitVerdict(paragraph: Paragraph, bold: Strong, name: string): Paragraph[] {
  markVerdict(bold, name);
  const label: Paragraph = {
    type: "paragraph",
    data: { hProperties: { className: ["verdict-line"] } },
    children: [bold],
  };

  const rest = paragraph.children.slice(1);
  const [next] = rest;
  if (next?.type === "text") next.value = next.value.trimStart();
  return rest.length === 0 ? [label] : [label, { type: "paragraph", children: rest }];
}

const verdicts: Plugin<[string], Mdast> = (file) => (tree) => {
  if (!VERDICT_PAGES.has(file)) return;

  const children: RootContent[] = [];
  let entry: { line: number | undefined; found: boolean } | undefined;
  const closeEntry = () => {
    if (entry !== undefined && !entry.found)
      throw new Error(`docs/${file}: the entry on line ${String(entry.line)} has no verdict`);
  };

  for (const node of tree.children) {
    if (node.type === "heading" && node.depth <= 3) {
      closeEntry();
      entry = node.depth === 3 ? { line: node.position?.start.line, found: false } : undefined;
    }

    const lead = node.type === "paragraph" ? node.children[0] : undefined;
    const text = boldText(lead);
    if (entry === undefined || node.type !== "paragraph" || lead?.type !== "strong" || !text?.endsWith(".")) {
      children.push(node);
      continue;
    }

    const name = text.slice(0, -1);
    if (!VERDICTS.has(name)) throw new Error(`docs/${file}: "${name}" isn't a verdict`);
    if (entry.found) throw new Error(`docs/${file}: the entry on line ${String(entry.line)} has two verdicts`);
    entry.found = true;
    children.push(...splitVerdict(node, lead, name));
  }
  closeEntry();
  tree.children = children;

  visit(tree, "tableCell", (cell) => {
    const [only, extra] = cell.children;
    const name = boldText(only);
    if (only?.type === "strong" && extra === undefined && name !== undefined && VERDICTS.has(name))
      markVerdict(only, name);
  });
};

// A PNG beside the page, `![Alt](install-app.png)`, works on GitHub as written.
// Here it gets its own stable route, the page's folder with the guide's prefix
// taken off, which the [...file=png] endpoint prerenders. The CSP allows images
// only from this origin, so anything else would fail silently in the browser:
// an image that isn't such a PNG, or has no alt text, fails the build instead.
function checkedPicture(url: string, alt: string | null | undefined, file: string): string {
  if (!/^[^/:#?]+\.png$/.test(url))
    throw new Error(
      `docs/${file}: image "${url}" must be a PNG named beside the page, such as ![Alt](install-app.png)`,
    );
  if ((alt ?? "").trim() === "") throw new Error(`docs/${file}: image "${url}" has no alt text`);

  const path = posix.join(posix.dirname(file), url);
  if (!pngs.has(path)) throw new Error(`docs/${file}: its image names docs/${path}, which doesn't exist`);
  return path;
}

function pictureBytes(path: string): Uint8Array<ArrayBuffer> {
  const encoded = pngs.get(path)?.split(",", 2)[1] ?? "";
  return new Uint8Array(Buffer.from(encoded, "base64"));
}

/** A PNG's size from its header, so the page doesn't shift when the image arrives. */
function pictureSize(path: string): { width: number; height: number } {
  const header = new DataView(pictureBytes(path).buffer);
  return { width: header.getUint32(16), height: header.getUint32(20) };
}

const rewritePictures: Plugin<[string, string[]], Mdast> = (file, routes) => (tree) => {
  visit(tree, (node) => {
    if (node.type === "imageReference")
      throw new Error(`docs/${file}: write the image "${node.identifier}" inline, as ![Alt](name.png)`);
    if (node.type !== "image") return;
    refuseTitle(node, file);

    const path = checkedPicture(node.url, node.alt, file);
    const route = atSiteRoot(path);
    routes.push(route);
    node.url = `/${route}`;
    node.data = { hProperties: { ...pictureSize(path), loading: "lazy", decoding: "async" } };
  });
};

// A mermaid block whose first line names its drawing, `%% svg: name.svg` (a
// Mermaid comment, so GitHub hides it), becomes that SVG beside the page.
// GitHub draws the Mermaid instead. The SVG is inlined so docs.css colors its
// classes from the tokens, and `cargo xtask check-repository` holds each pair
// to its hashes in docs/diagrams.lock.
const DRAWING = /^%% svg: (\S+\.svg)\s*$/;

function diagram(source: string, file: string): Element {
  const name = DRAWING.exec(source.split("\n", 1)[0] ?? "")?.[1];
  if (name === undefined) throw new Error(`docs/${file}: a mermaid block must start with "%% svg: <file>.svg"`);
  const path = posix.normalize(posix.join(posix.dirname(file), name));
  const svg = svgs.get(path);
  if (svg === undefined) throw new Error(`docs/${file}: its mermaid block names docs/${path}, which doesn't exist`);

  return {
    type: "element",
    tagName: "div",
    properties: { className: ["diagram"] },
    children: [{ type: "raw", value: svg.trim() }],
  };
}

// One theme whose colors are CSS variables, so code follows the page's light
// or dark tokens. The transformer turns each inline color into a class, since
// the site's CSP allows no inline styles; docs.css colors the classes.
const theme = createCssVariablesTheme({ name: "wellfed", variablePrefix: "--code-" });
const highlighter = await createHighlighter({
  themes: [theme],
  langs: ["shellscript", "rust", "toml", "json", "markdown", "http"],
});
const languages = new Set(highlighter.getLoadedLanguages());

const tokenClasses: ShikiTransformer = {
  pre(node) {
    delete node.properties.style;
    delete node.properties.tabindex;
  },
  span(node) {
    const style = node.properties.style;
    delete node.properties.style;
    const token = typeof style === "string" ? /var\(--code-(?:token-)?([a-z-]+)\)/.exec(style)?.[1] : undefined;
    if (token !== undefined && token !== "foreground") this.addClassToHast(node, `tok-${token}`);
  },
};

/** The language a fenced block declares, such as `rust` for ```` ```rust ````. */
function languageOf(pre: Element): string | undefined {
  const [code] = pre.children;
  if (code?.type !== "element" || code.tagName !== "code") return undefined;
  const classes = code.properties.className;
  return Array.isArray(classes)
    ? classes
        .map(String)
        .find((name) => name.startsWith("language-"))
        ?.slice("language-".length)
    : undefined;
}

function highlight(pre: Element): Element | undefined {
  const [code] = pre.children;
  if (code?.type !== "element" || code.tagName !== "code") return undefined;
  const declared = languageOf(pre);
  const lang = declared !== undefined && languages.has(declared) ? declared : "text";

  const root = highlighter.codeToHast(toString(code).replace(/\n$/, ""), {
    lang,
    theme: "wellfed",
    transformers: [tokenClasses],
  });
  const [highlighted] = root.children;
  if (highlighted?.type !== "element") return undefined;
  if (declared !== undefined) highlighted.properties.dataLang = declared;
  // Diagrams and plain output read as noise in a search excerpt.
  if (lang === "text") highlighted.properties.dataPagefindIgnore = "";
  return highlighted;
}

function excerpt(text: string): string {
  const flat = text.replace(/\s+/g, " ").trim();
  if (flat.length <= 160) return flat;
  return `${flat.slice(0, flat.lastIndexOf(" ", 157))}…`;
}

interface Collected {
  title: string;
  description: string;
  headings: Heading[];
}

// After headings have ids: collect the title, the first paragraph as the
// description and the h2/h3 outline, give each heading a self-link, draw
// diagrams, highlight code, and let wide tables scroll inside the column
// instead of the page.
const decorate: Plugin<[Collected, string], Hast> = (out, file) => (tree) => {
  visit(tree, "element", (node, index, parent) => {
    if (node.tagName === "h1" && out.title === "") out.title = toString(node);
    if (node.tagName === "p" && out.description === "" && parent?.type === "root")
      out.description = excerpt(toString(node));

    if (node.tagName === "h2" || node.tagName === "h3") {
      const id = typeof node.properties.id === "string" ? node.properties.id : "";
      out.headings.push({ depth: node.tagName === "h2" ? 2 : 3, id, text: toString(node) });
      node.children.push({
        type: "element",
        tagName: "a",
        properties: {
          className: ["anchor"],
          href: `#${id}`,
          ariaLabel: "Link to this section",
          dataPagefindIgnore: "",
        },
        children: [{ type: "text", value: "#" }],
      });
      return SKIP;
    }

    if (parent === undefined || index === undefined) return undefined;

    if (node.tagName === "pre") {
      const replaced = languageOf(node) === "mermaid" ? diagram(toString(node), file) : highlight(node);
      if (replaced !== undefined) parent.children[index] = replaced;
      return SKIP;
    }

    if (node.tagName === "table") {
      parent.children[index] = {
        type: "element",
        tagName: "div",
        properties: { className: ["table-scroll"] },
        children: [node],
      };
      return SKIP;
    }

    return undefined;
  });
};

/** One page's HTML. Exported for the tests; a page reaches readers only through `docs`. */
export function render(file: string, markdown: string): Doc {
  const out: Collected = { title: "", description: "", headings: [] };
  const images: string[] = [];
  const html = unified()
    .use(remarkParse)
    .use(remarkGfm)
    .use(rewriteLinks, file)
    .use(rewritePictures, file, images)
    .use(alerts)
    .use(verdicts, file)
    .use(remarkRehype)
    .use(rehypeSlug)
    .use(decorate, out, file)
    // Only the diagrams are raw HTML: remarkRehype drops any in the Markdown.
    .use(rehypeStringify, { allowDangerousHtml: true })
    .processSync(markdown);

  return {
    file,
    slug: routeOf(file),
    title: out.title || routeOf(file),
    description: out.description,
    html: String(html),
    headings: out.headings,
    images,
    editUrl: edit(`docs/${file}`),
  };
}

/**
 * A doc's Markdown as it reads away from the repository, for "Copy as
 * Markdown" and the `.md` file beside each page: every link and image made
 * absolute, by the same rule as the page (a published doc to its page on this
 * site, anything else to GitHub). All of it is found with remark, so a `](`
 * inside code is left alone. The source is the doc's own unless a test passes one.
 */
export function portableMarkdown(file: string, markdown = files.get(file)): string | undefined {
  if (markdown === undefined) return undefined;

  const edits: { start: number; end: number; text: string }[] = [];
  visit(unified().use(remarkParse).use(remarkGfm).parse(markdown), (node) => {
    const start = node.position?.start.offset;
    const end = node.position?.end.offset;
    if (start === undefined || end === undefined) return;

    if (node.type !== "link" && node.type !== "definition" && node.type !== "image") return;
    refuseTitle(node, file);

    // The URL is the last occurrence in the node's source: after the text of
    // an inline link, after the label of a definition.
    const at = markdown.lastIndexOf(node.url, end);
    if (at < start) return;
    const resolved =
      node.type === "image" ? `/${atSiteRoot(checkedPicture(node.url, node.alt, file))}` : resolveLink(node.url, file);
    edits.push({
      start: at,
      end: at + node.url.length,
      text: resolved.startsWith("/") ? `${SITE}${resolved}` : resolved,
    });
  });

  let out = markdown;
  for (const { start, end, text } of edits.sort((a, b) => b.start - a.start))
    out = `${out.slice(0, start)}${text}${out.slice(end)}`;
  return out;
}

/**
 * Routes against what publishes them, refusing two sources on one route: the
 * guide drops its prefix, so `guide/roadmap.md` and `roadmap.md` would otherwise
 * overwrite each other without a word. The route `index` is the root's own
 * `index.html` and `index.md`, so a doc can't take it.
 */
export function routeTable(entries: Iterable<readonly [route: string, source: string]>): Map<string, string> {
  const table = new Map<string, string>();
  for (const [route, source] of entries) {
    if (route === "index") throw new Error(`docs/${source} would publish /index, which is the root's file`);
    const taken = table.get(route);
    if (taken !== undefined && taken !== source)
      throw new Error(`docs/${taken} and docs/${source} both publish /${route}`);
    table.set(route, source);
  }
  return table;
}

/** Every published doc, keyed by its path under docs/. */
export const docs: ReadonlyMap<string, Doc> = new Map(
  [...files].filter(([file]) => published.has(file)).map(([file, markdown]) => [file, render(file, markdown)]),
);

routeTable([...docs.values()].map((doc) => [doc.slug, doc.file]));

export const bySlug: ReadonlyMap<string, Doc> = new Map([...docs.values()].map((doc) => [doc.slug, doc]));

// Only an image a published page shows is published. A PNG's route already names
// its folder, so its source is the route itself unless the guide's prefix came off.
const pictureSources = routeTable(
  [...docs.values()].flatMap((doc) =>
    doc.images.map((route) => [route, posix.join(posix.dirname(doc.file), posix.basename(route))] as const),
  ),
);

/** Each published PNG's bytes, keyed by its route without the leading slash. */
export const images: ReadonlyMap<string, Uint8Array<ArrayBuffer>> = new Map(
  [...pictureSources].map(([route, path]) => [route, pictureBytes(path)]),
);
