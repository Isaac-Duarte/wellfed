import { expect, test, type Page } from "@playwright/test";

const REPO_DOCS = "https://github.com/Isaac-Duarte/wellfed/blob/master/docs";

function collectErrors(page: Page): string[] {
  const errors: string[] = [];
  page.on("console", (message) => {
    if (message.type() === "error") errors.push(message.text());
  });
  return errors;
}

// The built site at desktop and phone width, served as the Worker serves it.
test("renders a doc from its own origin only", async ({ page, baseURL }) => {
  const origin = new URL(baseURL ?? "").origin;
  const offsite: string[] = [];
  const errors = collectErrors(page);
  page.on("request", (request) => {
    if (new URL(request.url()).origin !== origin) offsite.push(request.url());
  });

  await page.goto("/install");

  await expect(page.locator("h1")).toHaveCount(1);
  await expect(page.getByRole("link", { name: "Edit this page on GitHub" })).toHaveAttribute(
    "href",
    "https://github.com/Isaac-Duarte/wellfed/edit/master/docs/guide/install.md",
  );

  await page.waitForLoadState("networkidle");
  expect(offsite).toEqual([]);
  expect(errors).toEqual([]);
});

test("the sidebar is short, and every link in it opens", async ({ request, page, baseURL }) => {
  const origin = new URL(baseURL ?? "").origin;
  await page.goto("/");
  const hrefs = await page.locator("#sidebar a").evaluateAll((links) => links.map((a) => a.getAttribute("href") ?? ""));

  expect(hrefs.length).toBeGreaterThan(5);
  expect(hrefs.length).toBeLessThanOrEqual(15);

  const own = hrefs.filter((href) => new URL(href, origin).origin === origin);
  for (const href of own) expect((await request.get(href)).status(), href).toBe(200);
  expect(hrefs).toContain(`${REPO_DOCS}/development/README.md`);
});

test("an unpublished doc isn't served, and a link to it goes to GitHub", async ({ page, request }) => {
  expect((await request.get("/firmware/bank-policy")).status()).toBe(404);
  expect((await request.get("/firmware/bank-policy.md")).status()).toBe(404);

  await page.goto("/how-it-works/install-and-rollback");
  await expect(page.locator(".prose").getByRole("link", { name: "Bank policy" })).toHaveAttribute(
    "href",
    `${REPO_DOCS}/firmware/bank-policy.md`,
  );
});

test("search finds a published page and opens it", async ({ page }) => {
  await page.goto("/");
  await page.getByRole("button", { name: "Search docs" }).click();
  await page.getByRole("combobox").fill("roadmap");

  await expect(page.getByRole("option").first()).toBeVisible();
  await page.keyboard.press("Enter");
  await expect(page).not.toHaveURL(/\/$/);
  await expect(page.locator("h1")).toBeVisible();
});

test("search knows nothing of an unpublished page", async ({ page }) => {
  await page.goto("/");
  await page.getByRole("button", { name: "Search docs" }).click();

  // The motor driver's part number appears only on hardware pages left on GitHub.
  await page.getByRole("combobox").fill("TC118AH");
  await expect(page.getByText("Nothing matches")).toBeVisible();
  await expect(page.getByRole("option")).toHaveCount(0);
});

test.describe("App and CLI tabs", () => {
  test("show one panel and remember the choice across pages", async ({ page }) => {
    const errors = collectErrors(page);
    await page.goto("/install");

    const app = page.locator('.tab-panel[data-tab-panel="app"]').first();
    const cli = page.locator('.tab-panel[data-tab-panel="cli"]').first();
    await expect(app).toBeVisible();
    await expect(cli).toBeHidden();

    await page.getByRole("tab", { name: "CLI" }).first().click();
    await expect(cli).toBeVisible();
    await expect(app).toBeHidden();
    await expect(page.getByRole("tab", { name: "CLI" }).first()).toHaveAttribute("aria-selected", "true");

    await page.goto("/roadmap");
    await page.goto("/install");
    await expect(cli).toBeVisible();
    await expect(app).toBeHidden();
    expect(errors).toEqual([]);
  });

  test("move with the arrow keys", async ({ page }) => {
    await page.goto("/install");
    await page.getByRole("tab", { name: "App" }).first().focus();

    await page.keyboard.press("ArrowRight");

    await expect(page.getByRole("tab", { name: "CLI" }).first()).toBeFocused();
    await expect(page.locator('.tab-panel[data-tab-panel="cli"]').first()).toBeVisible();
  });

  test("a link into the other panel opens it", async ({ page }) => {
    await page.goto("/install#cli");

    await expect(page.locator('.tab-panel[data-tab-panel="cli"]').first()).toBeVisible();
    await expect(page.locator('.tab-panel[data-tab-panel="app"]').first()).toBeHidden();
  });

  test("without JavaScript both show, with no switch", async ({ browser, baseURL }) => {
    const context = await browser.newContext({ javaScriptEnabled: false, baseURL });
    const page = await context.newPage();

    await page.goto("/install");

    await expect(page.locator('.tab-panel[data-tab-panel="app"]').first()).toBeVisible();
    await expect(page.locator('.tab-panel[data-tab-panel="cli"]').first()).toBeVisible();
    await expect(page.locator(".tab-bar").first()).toBeHidden();
    await context.close();
  });
});

test("a page's image loads and fits the column", async ({ page }) => {
  await page.goto("/install");
  const image = page.locator(".prose img").first();

  await image.scrollIntoViewIfNeeded();
  await expect(image).toBeVisible();
  await expect.poll(() => image.evaluate((el: HTMLImageElement) => el.naturalWidth)).toBeGreaterThan(0);

  const fits = await image.evaluate((el) => el.getBoundingClientRect().width <= (el.parentElement?.clientWidth ?? 0));
  expect(fits).toBe(true);
});

test("copies a page as Markdown with absolute links", async ({ page, context, baseURL }) => {
  await context.grantPermissions(["clipboard-read", "clipboard-write"], { origin: new URL(baseURL ?? "").origin });
  await page.goto("/before-you-start");

  await page.getByRole("button", { name: "Copy as Markdown" }).click();
  await expect(page.getByRole("button", { name: "Copied" })).toBeVisible();

  const copied = await page.evaluate(() => navigator.clipboard.readText());
  expect(copied).toMatch(/^# Before you start/);
  expect(copied).toContain("](https://docs.well-fed.dev/devices)");
});

test("serves each page's Markdown beside it", async ({ request }) => {
  const page = await request.get("/roadmap.md");
  expect(page.status()).toBe(200);
  expect(page.headers()["content-type"]).toContain("text/markdown");
  expect(await page.text()).toMatch(/^# Roadmap/);

  expect((await request.get("/index.md")).status()).toBe(200);
});

test("never scrolls sideways", async ({ page }) => {
  for (const path of ["/", "/install", "/roadmap", "/server/running", "/devices"]) {
    await page.goto(path);
    const overflow = await page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth);
    expect(overflow, path).toBeLessThanOrEqual(0);
  }
});
