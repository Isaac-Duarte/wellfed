// The shape of a rendered doc, shared by the build-time renderer and the pages.
export interface Heading {
  depth: 2 | 3;
  id: string;
  text: string;
}

export interface Doc {
  /** The path under docs/, such as `guide/install.md`. */
  file: string;
  /** The route without its leading slash; a folder's README is the folder. */
  slug: string;
  title: string;
  description: string;
  html: string;
  headings: Heading[];
  /** The route of each PNG the page shows, without the leading slash. */
  images: string[];
  editUrl: string;
}
