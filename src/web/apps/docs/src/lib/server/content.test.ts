import { describe, expect, test } from "vitest";
import { docs, images, portableMarkdown, render, sourceOf } from "#lib/server/content.ts";

// Rendering a doc the site doesn't publish: these rules hold for any page, and
// the drawings live on hardware pages that stay on GitHub.
function rendered(file: string) {
  const markdown = sourceOf(file);
  if (markdown === undefined) throw new Error(`no doc ${file}`);
  return render(file, markdown);
}

describe("publishing", () => {
  test("an unlisted doc is not rendered", () => {
    expect(docs.has("firmware/motor-and-feeds.md")).toBe(false);
    expect(docs.has("development/rust.md")).toBe(false);
  });
});

describe("diagrams", () => {
  // The pages own their drawings and may move them; the rules are pinned with
  // Markdown of the test's own, naming drawings that live in docs/.
  const drawing = (name: string) => `\`\`\`mermaid\n%% svg: ${name}\nflowchart LR\n  a --> b\n\`\`\`\n`;

  test("a mermaid block becomes the drawing it names, inline", () => {
    const { html } = render(
      "devices/plaf103/test.md",
      `# Page\n\n${drawing("food-path.svg")}\n${drawing("parts.svg")}`,
    );

    expect(html).toContain('<div class="diagram"><svg xmlns="http://www.w3.org/2000/svg"');
    expect(html).toContain('<title id="food-path-title">');
    expect(html).toContain('<title id="parts-title">');
    expect(html).not.toContain("language-mermaid");
  });

  test("pages in different folders draw the same file", () => {
    for (const [file, name] of [
      ["devices/plaf103/test.md", "../../firmware/boot-flow.svg"],
      ["firmware/test.md", "boot-flow.svg"],
    ] as const)
      expect(render(file, drawing(name)).html, file).toContain('<title id="boot-flow-title">');
  });

  test.each([
    ["no drawing named", "```mermaid\nflowchart LR\n  a --> b\n```\n", 'must start with "%% svg: <file>.svg"'],
    ["a missing drawing", drawing("nope.svg"), "which doesn't exist"],
  ])("%s fails the build", (_name, markdown, message) => {
    expect(() => render("firmware/test.md", markdown)).toThrow(message);
  });

  test("a mermaid fence quoted in a longer one stays code", () => {
    const html = render(
      "development/test.md",
      `\`\`\`\`markdown\n${drawing("../firmware/boot-flow.svg")}\`\`\`\`\n`,
    ).html;

    expect(html).toContain("%% svg: ../firmware/boot-flow.svg");
    expect(html).not.toContain('class="diagram"');
  });
});

describe("verdicts", () => {
  const LOG = "guide/how-it-works/testing.md";

  test("a verdict becomes a label above its findings, without its full stop", () => {
    const { html } = render(LOG, "# Log\n\n### A run\n\n**Passed with gaps.** It held.\n");

    expect(html).toContain(
      '<p class="verdict-line"><strong class="verdict verdict-gaps">Passed with gaps</strong></p>\n<p>It held.</p>',
    );
  });

  test("a verdict with nothing after it leaves no empty paragraph", () => {
    const { html } = render(LOG, "# Log\n\n### A run\n\n**Open.**\n\n- Still running.\n");

    expect(html).toContain('<strong class="verdict verdict-open">Open</strong></p>\n<ul>');
    expect(html).not.toContain("<p></p>");
  });

  test("a table cell holding only a verdict is marked, and other bold isn't", () => {
    const { html } = render(LOG, "# Log\n\n| A | B |\n| --- | --- |\n| **Partly fixed** | **Mostly fine** |\n");

    expect(html).toContain('<td><strong class="verdict verdict-partly">Partly fixed</strong></td>');
    expect(html).toContain("<td><strong>Mostly fine</strong></td>");
  });

  test("another page's bold stays bold", () => {
    expect(render("guide/test.md", "### A run\n\n**Open.** It held.\n").html).not.toContain("verdict");
  });

  test("every entry in the real log has one", () => {
    const { html, headings } = rendered(LOG);
    const entries = headings.filter((heading) => heading.depth === 3);

    expect(entries.length).toBeGreaterThan(0);
    expect(html.match(/class="verdict-line"/g)).toHaveLength(entries.length);
  });

  test.each([
    ["no verdict", "### A run\n\nIt held.\n\n### Another\n\n**Open.**\n", "line 1 has no verdict"],
    [
      "the last entry's missing verdict",
      "### A run\n\n**Open.**\n\n### Another\n\nIt held.\n",
      "line 5 has no verdict",
    ],
    ["a phrase outside the set", "### A run\n\n**Mostly fine.** It held.\n", '"Mostly fine" isn\'t a verdict'],
    ["two verdicts", "### A run\n\n**Open.**\n\n**Passed.**\n", "has two verdicts"],
  ])("%s fails the build", (_name, markdown, message) => {
    expect(() => render(LOG, markdown)).toThrow(message);
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

  test("an image is absolute", () => {
    const install = portableMarkdown("guide/install.md") ?? "";

    expect(install).toContain("(https://docs.well-fed.dev/app-installing.png)");
  });

  test("code is left alone", () => {
    const source = "```mermaid\n%% svg: food-path.svg\nflowchart LR\n  a[link.md] --> b\n```\n";

    expect(portableMarkdown("devices/plaf103/test.md", source)).toBe(source);
  });

  test.each([
    ["a link", '[a](../roadmap.md "../roadmap.md")'],
    ["a definition", '[a]\n\n[a]: ../roadmap.md "../roadmap.md"'],
    ["an image", '![A picture](app-installing.png "app-installing.png")'],
  ])("%s with a title fails the build", (_name, markdown) => {
    expect(() => portableMarkdown("guide/test.md", markdown)).toThrow("has a title");
    expect(() => render("guide/test.md", markdown)).toThrow("has a title");
  });

  test("a file that isn't a doc has none", () => {
    expect(portableMarkdown("nope.md")).toBeUndefined();
  });
});
