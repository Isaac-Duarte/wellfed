import { describe, expect, test } from "vitest";
import { docs, images, portableMarkdown, render, sourceOf } from "#lib/server/content.ts";

// Rendering a doc the site doesn't publish: these rules hold for any page, and
// the drawings live on hardware pages that stay on GitHub.
function rendered(file: string) {
  const markdown = sourceOf(file);
  if (markdown === undefined) throw new Error(`no doc ${file}`);
  return render(file, markdown);
}

const TABS = "<!-- tabs -->\n\n### App\n\nTap it.\n\n### CLI\n\nRun it.\n\n<!-- /tabs -->\n";

describe("publishing", () => {
  test("an unlisted doc is not rendered", () => {
    expect(docs.has("firmware/motor-pulse.md")).toBe(false);
    expect(docs.has("development/rust.md")).toBe(false);
  });
});

describe("diagrams", () => {
  test("a mermaid block becomes the drawing it names, inline", () => {
    const html = rendered("devices/plaf103/README.md").html;

    expect(html).toContain('<div class="diagram"><svg xmlns="http://www.w3.org/2000/svg"');
    expect(html).toContain('<title id="food-path-title">');
    expect(html).toContain('<title id="parts-title">');
    expect(html).not.toContain("language-mermaid");
  });

  test("pages in different folders draw the same file", () => {
    for (const file of ["devices/plaf103/flash-layout.md", "firmware/bank-policy.md"])
      expect(rendered(file).html, file).toContain('<title id="boot-flow-title">');
  });

  test("a mermaid fence quoted in a longer one stays code", () => {
    const html = rendered("development/writing.md").html;

    expect(html).toContain("%% svg: boot-flow.svg");
    expect(html).not.toContain('class="diagram"');
  });
});

describe("tabs", () => {
  test("each section becomes a panel under a tab", () => {
    const { html } = render("guide/test.md", `# Page\n\n${TABS}`);

    expect(html).toContain('role="tablist"');
    expect(html).toContain('data-tab-panel="app"');
    expect(html).toContain('data-tab-panel="cli"');
    expect(html).toContain("Tap it.");
    expect(html).toContain("Run it.");
  });

  test("the tab headings stay out of the outline", () => {
    const { headings } = render("guide/test.md", `# Page\n\n## Steps\n\n${TABS}`);

    expect(headings.map((h) => h.text)).toEqual(["Steps"]);
  });

  test.each([
    ["no closing marker", "<!-- tabs -->\n\n### App\n\nA\n\n### CLI\n\nB\n", "without a <!-- /tabs -->"],
    ["no opening marker", "### App\n\nA\n\n<!-- /tabs -->\n", "without a <!-- tabs -->"],
    ["a nested marker", "<!-- tabs -->\n\n<!-- tabs -->\n\n<!-- /tabs -->\n", "inside another"],
    ["a misspelled marker", "<!-- tab -->\n", "isn't a tabs marker"],
    ["the wrong sections", "<!-- tabs -->\n\n### CLI\n\nB\n\n### App\n\nA\n\n<!-- /tabs -->\n", 'exactly "### App"'],
    ["the wrong level", "<!-- tabs -->\n\n## App\n\nA\n\n## CLI\n\nB\n\n<!-- /tabs -->\n", "level 3"],
    ["an empty section", "<!-- tabs -->\n\n### App\n\n### CLI\n\nB\n\n<!-- /tabs -->\n", "empty section"],
  ])("%s fails the build", (_name, markdown, message) => {
    expect(() => render("guide/test.md", markdown)).toThrow(message);
  });
});

describe("images", () => {
  test("a published page's PNG gets a route without the guide's prefix", () => {
    const { html, images: routes } = rendered("guide/install.md");

    expect(routes).toContain("app-installing.png");
    expect(html).toContain('src="/app-installing.png"');
    expect(html).toMatch(/width="\d+" height="\d+"/);
    expect(images.get("app-installing.png")?.slice(1, 4)).toEqual(new TextEncoder().encode("PNG"));
  });

  test.each([
    ["a missing file", "![A picture](nope.png)", "which doesn't exist"],
    ["no alt text", "![](app-installing.png)", "has no alt text"],
    ["another format", "![A picture](photo.jpg)", "must be a PNG"],
    ["another origin", "![A picture](https://example.com/a.png)", "must be a PNG"],
    ["a reference", "![A picture][a]\n\n[a]: app-installing.png", "inline"],
  ])("%s fails the build", (_name, markdown, message) => {
    expect(() => render("guide/test.md", markdown)).toThrow(message);
  });
});

describe("portable markdown", () => {
  const markdown = portableMarkdown("devices/plaf103/README.md") ?? "";

  test("links point at the site or GitHub, not the repository's files", () => {
    expect(markdown).toContain(
      "[RTL8721DM](https://github.com/Isaac-Duarte/wellfed/blob/master/docs/chips/rtl8721dm/README.md)",
    );
    expect(markdown).toContain("(https://github.com/Isaac-Duarte/wellfed/blob/master/src/firmware/devices/plaf103)");
    expect(markdown).not.toMatch(/\]\((?!https?:|#)/);
  });

  test("a published doc's link points at its page", () => {
    expect(portableMarkdown("guide/before-you-start.md")).toContain("(https://docs.well-fed.dev/devices)");
  });

  test("an image is absolute, the tabs markers are dropped and both sections stay", () => {
    const install = portableMarkdown("guide/install.md") ?? "";

    expect(install).toContain("(https://docs.well-fed.dev/app-installing.png)");
    expect(install).not.toContain("<!-- tabs -->");
    expect(install).not.toContain("<!-- /tabs -->");
    expect(install).toContain("### App");
    expect(install).toContain("### CLI");
  });

  test("code is left alone", () => {
    expect(markdown).toContain("%% svg: food-path.svg");
    expect(portableMarkdown("development/writing.md")).toContain("%% svg: boot-flow.svg");
  });

  test("a file that isn't a doc has none", () => {
    expect(portableMarkdown("nope.md")).toBeUndefined();
  });
});
