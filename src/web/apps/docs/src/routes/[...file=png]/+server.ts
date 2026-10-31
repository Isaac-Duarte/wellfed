import { error } from "@sveltejs/kit";
import { images } from "#lib/server/content.ts";
import type { EntryGenerator, RequestHandler } from "./$types";

// Each PNG a published page shows, prerendered beside the page. An endpoint
// doesn't inherit the layout's prerender.
export const prerender = true;
export const entries: EntryGenerator = () => [...images.keys()].map((file) => ({ file }));

export const GET: RequestHandler = ({ params }) => {
  const bytes = images.get(params.file);
  if (bytes === undefined) error(404, "No such image");

  return new Response(bytes, { headers: { "content-type": "image/png" } });
};
