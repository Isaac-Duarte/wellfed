// Serves an app's build/ as its Cloudflare Worker does (wrangler.jsonc's
// "auto-trailing-slash"): /page answers with page.html, a folder with its
// index.html, and every response carries the headers build/_headers gives its
// path. Both apps' Playwright runs use it, so the production CSP, most of which
// lives only in _headers, holds in every test. `vite preview` can't stand in:
// it applies no _headers and serves SvelteKit's own output, not the Pagefind
// index written into build/.
//
//   node ../../scripts/serve.ts   (from an app's folder, after its build)
import { createReadStream, readFileSync, statSync } from "node:fs";
import { createServer } from "node:http";
import { extname, join, normalize } from "node:path";

const root = join(process.cwd(), "build");
const port = Number(process.env.PORT ?? 4173);
const host = process.env.HOST ?? "127.0.0.1";

const TYPES: Record<string, string> = {
  ".html": "text/html; charset=utf-8",
  ".js": "text/javascript",
  ".css": "text/css",
  ".json": "application/json",
  ".svg": "image/svg+xml",
  ".png": "image/png",
  ".md": "text/markdown; charset=utf-8",
  ".woff2": "font/woff2",
  ".wasm": "application/wasm",
  ".txt": "text/plain; charset=utf-8",
};

interface Rule {
  pattern: string;
  headers: [name: string, value: string][];
}

// Cloudflare's format: an unindented path pattern, then its indented
// `Name: value` lines. Only a trailing `*` is a wildcard here, which is all
// either app's file uses; anything else fails rather than match differently.
function parseHeaders(text: string): Rule[] {
  const rules: Rule[] = [];
  for (const line of text.split("\n")) {
    if (line.trim() === "" || line.trimStart().startsWith("#")) continue;
    if (!/^\s/.test(line)) {
      const pattern = line.trim();
      if (!pattern.startsWith("/") || pattern.slice(0, -1).includes("*"))
        throw new Error(`_headers: unsupported pattern "${pattern}"`);
      rules.push({ pattern, headers: [] });
      continue;
    }
    const rule = rules.at(-1);
    const at = line.indexOf(":");
    if (rule === undefined || at < 0) throw new Error(`_headers: "${line.trim()}" belongs to no path`);
    rule.headers.push([line.slice(0, at).trim(), line.slice(at + 1).trim()]);
  }
  return rules;
}

function headersFor(rules: Rule[], pathname: string): Record<string, string> {
  const out: Record<string, string> = {};
  for (const { pattern, headers } of rules) {
    const matches = pattern.endsWith("*") ? pathname.startsWith(pattern.slice(0, -1)) : pathname === pattern;
    if (matches) for (const [name, value] of headers) out[name.toLowerCase()] = value;
  }
  return out;
}

function isFile(path: string): boolean {
  try {
    return statSync(path).isFile();
  } catch {
    return false;
  }
}

function resolve(pathname: string): string | undefined {
  const path = join(root, normalize(decodeURIComponent(pathname)));
  if (!path.startsWith(root)) return undefined;
  return [path, `${path}.html`, join(path, "index.html")].find(isFile);
}

const rules = parseHeaders(readFileSync(join(root, "_headers"), "utf8"));

createServer((request, response) => {
  const pathname = new URL(request.url ?? "/", "http://localhost").pathname;
  const file = resolve(pathname);
  if (file === undefined) {
    response.writeHead(404).end("Not found");
    return;
  }
  response.writeHead(200, {
    ...headersFor(rules, pathname),
    "content-type": TYPES[extname(file)] ?? "application/octet-stream",
  });
  createReadStream(file).pipe(response);
}).listen(port, host, () => {
  console.log(`serving ${root} on http://${host}:${port}/`);
});
