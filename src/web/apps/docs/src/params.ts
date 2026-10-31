import { defineParams } from "@sveltejs/kit/params";

// Route parameter matchers. `markdown` is a page's Markdown twin,
// `/devices.md`, and `png` an image beside a page, `/how-it-works/x.png`. Their
// routes outrank the pages' own `[...slug]` because they have a matcher. The value is the segment itself, since Kit
// rebuilds the URL from it with String().
export const params = defineParams({
  markdown: (param: string) => (param.endsWith(".md") ? param : undefined),
  png: (param: string) => (param.endsWith(".png") ? param : undefined),
});
