import { describe, expect, test } from "vitest";
import { NAV, PUBLISHED, isPage } from "#lib/nav.ts";
import { docs, resolveLink, routeOf, routeTable, sourceFiles } from "#lib/server/content.ts";

describe("the sidebar", () => {
  test("every entry names a doc that exists", () => {
    const named = NAV.flatMap((group) => group.entries).map((entry) => (isPage(entry) ? entry.file : entry.onGitHub));

    for (const file of named) expect(sourceFiles, file).toContain(file);
  });

  test("lists a file once", () => {
    expect(new Set(PUBLISHED).size).toBe(PUBLISHED.length);
  });

  test("lists every page of the guide", () => {
    const guide = sourceFiles.filter((file) => file.startsWith("guide/"));

    expect(guide.length).toBeGreaterThan(0);
    for (const file of guide) expect(PUBLISHED, file).toContain(file);
  });

  test("publishes what it lists and nothing else", () => {
    expect([...docs.keys()].sort()).toEqual([...PUBLISHED].sort());
  });
});

describe("routes", () => {
  test("the guide is the site's root", () => {
    expect(routeOf("guide/README.md")).toBe("");
    expect(routeOf("guide/install.md")).toBe("install");
    expect(routeOf("guide/how-it-works/install-and-rollback.md")).toBe("how-it-works/install-and-rollback");
  });

  test("other docs keep their path", () => {
    expect(routeOf("roadmap.md")).toBe("roadmap");
    expect(routeOf("devices/README.md")).toBe("devices");
    expect(routeOf("server/running.md")).toBe("server/running");
  });

  test("two files on one route are refused", () => {
    expect(() =>
      routeTable([
        ["roadmap", "roadmap.md"],
        ["roadmap", "guide/roadmap.md"],
      ]),
    ).toThrow("docs/roadmap.md and docs/guide/roadmap.md both publish /roadmap");
  });
});

describe("links", () => {
  test("a published doc becomes its route, with its anchor", () => {
    expect(resolveLink("../../roadmap.md#feeding", "guide/how-it-works/x.md")).toBe("/roadmap#feeding");
    expect(resolveLink("../devices/README.md", "guide/install.md")).toBe("/devices");
    expect(resolveLink("README.md", "guide/install.md")).toBe("/");
  });

  test("an unpublished doc goes to GitHub, with its anchor", () => {
    expect(resolveLink("../firmware/pairing.md#keys", "guide/install.md")).toBe(
      "https://github.com/Isaac-Duarte/wellfed/blob/master/docs/firmware/pairing.md#keys",
    );
    expect(resolveLink("../devices/plaf103/README.md", "guide/install.md")).toBe(
      "https://github.com/Isaac-Duarte/wellfed/blob/master/docs/devices/plaf103/README.md",
    );
  });

  test("a doc that doesn't exist fails the build", () => {
    expect(() => resolveLink("../firmware/nope.md", "guide/install.md")).toThrow("doesn't exist");
  });

  test("a link outside docs/ goes to GitHub", () => {
    expect(resolveLink("../../deny.toml", "development/rust.md")).toBe(
      "https://github.com/Isaac-Duarte/wellfed/blob/master/deny.toml",
    );
  });

  test("anchors and absolute URLs are left alone", () => {
    expect(resolveLink("#evidence", "firmware/qa.md")).toBe("#evidence");
    expect(resolveLink("https://example.com/a.md", "firmware/qa.md")).toBe("https://example.com/a.md");
  });
});
