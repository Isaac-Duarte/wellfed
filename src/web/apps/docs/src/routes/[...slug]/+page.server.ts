import { error } from "@sveltejs/kit";
import { bySlug } from "#lib/server/content.ts";
import { placeOf } from "#lib/server/site.ts";
import type { EntryGenerator, PageServerLoad } from "./$types";

// Only a published doc is a page: a link to any other goes to GitHub.
export const entries: EntryGenerator = () => [...bySlug.keys()].map((slug) => ({ slug }));

export const load: PageServerLoad = ({ params }) => {
  const doc = bySlug.get(params.slug);
  if (doc === undefined) error(404, "No such page");

  return { doc, ...placeOf(doc.slug) };
};
