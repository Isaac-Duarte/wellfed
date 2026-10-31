// The docs nav.ts lists, rendered to HTML at build time. The Markdown stays
// plain GitHub-flavored Markdown that reads the same on GitHub; this module is
// the one place that turns it into site pages: relative links become site
// routes or GitHub URLs, code is highlighted, and each page's headings feed
// its table of contents. It runs only during prerender, so none of it ships.
// A doc nav.ts doesn't list stays on GitHub and is never rendered or searched.
import { posix } from "node:path";
import type { Element, ElementContent, Root as Hast } from "hast";
import { toString } from "hast-util-to-string";
import type { Root as Mdast } from "mdast";
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

const rewriteLinks: Plugin<[string], Mdast> = (file) => (tree) => {
  visit(tree, (node) => {
    if (node.type === "link" || node.type === "definition") node.url = resolveLink(node.url, file);
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

    const path = checkedPicture(node.url, node.alt, file);
    const route = atSiteRoot(path);
    routes.push(route);
    node.url = `/${route}`;
    node.data = { hProperties: { ...pictureSize(path), loading: "lazy", decoding: "async" } };
  });
};

// App | CLI tabs: `<!-- tabs -->` and `<!-- /tabs -->` on their own lines around
// plain `### App` and `### CLI` sections. GitHub hides the comments and shows
// both in order, and so does the site without JavaScript: only docs.css hides a
// panel, and only under the root's data-tab, which tabs.ts sets. Pagefind reads
// the built HTML and not the CSS, so it indexes both. bits-ui's Tabs isn't used
// because it decides what's visible from its own state, which needs JavaScript.
const TABS_OPEN = "<!-- tabs -->";
const TABS_CLOSE = "<!-- /tabs -->";
const NEAR_MARKER = /^<!--.*\btabs?\b.*-->$/is;

const isTabsMarker = (html: string) => html.trim() === TABS_OPEN || html.trim() === TABS_CLOSE;

const markTabs: Plugin<[string], Mdast> = (file) => (tree) => {
  visit(tree, "html", (node, index, parent) => {
    if (!NEAR_MARKER.test(node.value.trim())) return;
    if (!isTabsMarker(node.value))
      throw new Error(`docs/${file}: "${node.value}" isn't a tabs marker: write ${TABS_OPEN} or ${TABS_CLOSE}`);
    if (parent !== tree || index === undefined)
      throw new Error(`docs/${file}: ${node.value} must be on its own line at the top level of the page`);

    const dataTabs = node.value.trim() === TABS_OPEN ? "open" : "close";
    parent.children[index] = { type: "thematicBreak", data: { hProperties: { dataTabs } } };
  });
};

const TAB_NAMES = [
  { label: "App", tab: "app" },
  { label: "CLI", tab: "cli" },
] as const;

const TAB_HEADING = "tab-heading";

function isTabHeading(node: Element): boolean {
  return Array.isArray(node.properties.className) && node.properties.className.includes(TAB_HEADING);
}

function isHeading(node: ElementContent): node is Element {
  return node.type === "element" && /^h[1-3]$/.test(node.tagName);
}

// One set of panels, built here because the Markdown holds their content. The
// tablist's state in this HTML is the default; tabs.ts corrects it on hydration.
function tabSet(content: ElementContent[], file: string, set: number): Element {
  const blocks = content.filter((node) => node.type !== "text" || node.value.trim() !== "");
  const headings = blocks.filter(isHeading);
  const names = headings.map((heading) => toString(heading));
  const first = headings[0];
  const second = headings[1];
  const fails = (why: string) => new Error(`docs/${file}: tabs ${String(set)} ${why}`);

  if (first === undefined || second === undefined || names.join() !== "App,CLI" || blocks[0] !== first)
    throw fails(`must hold exactly "### App" then "### CLI", and starts with: ${names.join(", ") || "no heading"}`);
  if (first.tagName !== "h3" || second.tagName !== "h3")
    throw fails('must use level 3 headings, "### App" and "### CLI"');

  const middle = blocks.indexOf(second);
  const sections = [blocks.slice(0, middle), blocks.slice(middle)];
  if (sections.some((section) => section.length < 2)) throw fails("has an empty section");

  for (const heading of headings) heading.properties.className = [TAB_HEADING];

  const panels = TAB_NAMES.map(({ tab }, at): Element => ({
    type: "element",
    tagName: "div",
    properties: {
      className: ["tab-panel"],
      role: "tabpanel",
      id: `panel-${String(set)}-${tab}`,
      ariaLabelledBy: [`tab-${String(set)}-${tab}`],
      dataTabPanel: tab,
    },
    children: sections[at] ?? [],
  }));

  return {
    type: "element",
    tagName: "div",
    properties: { className: ["tabs"] },
    children: [
      {
        type: "element",
        tagName: "div",
        properties: { className: ["tab-bar"], role: "tablist", ariaLabel: "App or CLI", dataPagefindIgnore: "" },
        children: TAB_NAMES.map(({ label, tab }, at) => ({
          type: "element",
          tagName: "button",
          properties: {
            type: "button",
            className: ["tab"],
            role: "tab",
            id: `tab-${String(set)}-${tab}`,
            ariaControls: [`panel-${String(set)}-${tab}`],
            ariaSelected: at === 0 ? "true" : "false",
            tabIndex: at === 0 ? 0 : -1,
            dataTab: tab,
          },
          children: [{ type: "text", value: label }],
        })),
      },
      ...panels,
    ],
  };
}

const wrapTabs: Plugin<[string], Hast> = (file) => (tree) => {
  const out: Hast["children"] = [];
  let open: ElementContent[] | undefined;
  let sets = 0;

  for (const node of tree.children) {
    const marker = node.type === "element" ? node.properties.dataTabs : undefined;
    if (marker === "open") {
      if (open !== undefined) throw new Error(`docs/${file}: ${TABS_OPEN} inside another ${TABS_OPEN}`);
      open = [];
    } else if (marker === "close") {
      if (open === undefined) throw new Error(`docs/${file}: ${TABS_CLOSE} without a ${TABS_OPEN} before it`);
      sets += 1;
      out.push(tabSet(open, file, sets));
      open = undefined;
    } else if (open === undefined) {
      out.push(node);
    } else if (node.type !== "doctype") {
      open.push(node);
    }
  }

  if (open !== undefined) throw new Error(`docs/${file}: ${TABS_OPEN} without a ${TABS_CLOSE} after it`);
  tree.children = out;
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

    // A tab's own heading names its panel: the tab says it already, and App, CLI,
    // App, CLI in the outline would lead nowhere useful.
    if (isTabHeading(node)) return SKIP;

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
    .use(markTabs, file)
    .use(remarkRehype)
    .use(rehypeSlug)
    .use(wrapTabs, file)
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
 * site, anything else to GitHub), and the tabs markers dropped, so both
 * sections read in order. All of it is found with remark, so a `](` or a marker
 * inside code is left alone.
 */
export function portableMarkdown(file: string): string | undefined {
  const markdown = files.get(file);
  if (markdown === undefined) return undefined;

  const edits: { start: number; end: number; text: string }[] = [];
  visit(unified().use(remarkParse).use(remarkGfm).parse(markdown), (node) => {
    const start = node.position?.start.offset;
    const end = node.position?.end.offset;
    if (start === undefined || end === undefined) return;

    if (node.type === "html" && isTabsMarker(node.value)) {
      const blank = /^\n*/.exec(markdown.slice(end))?.[0].length ?? 0;
      edits.push({ start, end: end + blank, text: "" });
      return;
    }
    if (node.type !== "link" && node.type !== "definition" && node.type !== "image") return;

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
 * overwrite each other without a word.
 */
export function routeTable(entries: Iterable<readonly [route: string, source: string]>): Map<string, string> {
  const table = new Map<string, string>();
  for (const [route, source] of entries) {
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
