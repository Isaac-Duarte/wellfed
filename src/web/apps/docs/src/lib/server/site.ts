// The sidebar and reading order resolved against the rendered docs, for the
// layout and for each page's group label and previous and next links.
import { NAV, isPage, onGitHubUrl } from "#lib/nav.ts";
import { docs, routeOf } from "#lib/server/content.ts";

export interface PageLink {
  /** The route without its leading slash, for `resolve("/[...slug]", { slug })`. */
  slug: string;
  label: string;
}

export interface ExternalLink {
  href: string;
  label: string;
}

export type NavLink = PageLink | ExternalLink;

export interface NavSection {
  label: string;
  links: NavLink[];
}

export const nav: NavSection[] = NAV.map((group) => ({
  label: group.label,
  links: group.entries.map((entry) =>
    isPage(entry)
      ? { slug: routeOf(entry.file), label: entry.label ?? docs.get(entry.file)?.title ?? entry.file }
      : { href: onGitHubUrl(entry), label: entry.label },
  ),
}));

// Previous and next walk the site's own pages, so an external entry never has one.
const order = nav.flatMap((section) =>
  section.links.flatMap((link) => ("slug" in link ? [{ ...link, section: section.label }] : [])),
);

/** Where a page sits in the reading order: its group, and the pages either side. */
export function placeOf(slug: string) {
  const at = order.findIndex((link) => link.slug === slug);
  return {
    section: order[at]?.section,
    previous: at > 0 ? order[at - 1] : undefined,
    next: at >= 0 ? order[at + 1] : undefined,
  };
}
