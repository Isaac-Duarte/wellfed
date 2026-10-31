// The sidebar, and the only list of what the site publishes: a file listed here
// is rendered, searched and given a `.md` copy. Any other doc stays on GitHub,
// and links to it go there. A page's label defaults to its first heading; `label`
// gives a shorter one where the heading is long. nav.test.ts fails when an entry
// names a file that doesn't exist, when a file is listed twice, and when a file
// under docs/guide/ is left out.
import { blob } from "#lib/links.ts";

export interface NavPage {
  /** The path under docs/, such as `guide/install.md`. */
  file: string;
  label?: string;
}

/** A doc the site links to on GitHub, instead of publishing. */
export interface NavOnGitHub {
  /** The path under docs/, such as `development/README.md`. */
  onGitHub: string;
  label: string;
}

export type NavEntry = NavPage | NavOnGitHub;

export interface NavGroup {
  label: string;
  entries: NavEntry[];
}

export const NAV: NavGroup[] = [
  {
    label: "Start",
    entries: [
      { file: "guide/README.md", label: "Overview" },
      { file: "guide/before-you-start.md", label: "Before you start" },
      { file: "guide/install.md", label: "Install" },
      { file: "guide/everyday-use.md", label: "Everyday use" },
      { file: "guide/going-back.md", label: "Going back to stock" },
    ],
  },
  {
    label: "How it works",
    entries: [
      { file: "guide/how-it-works/system.md", label: "The system at a glance" },
      { file: "guide/how-it-works/install-and-rollback.md", label: "Install and rollback" },
      { file: "guide/how-it-works/feeding-safety.md", label: "How feeding stays safe" },
      { file: "guide/how-it-works/access.md", label: "Who can talk to your feeder" },
    ],
  },
  {
    label: "Reference",
    entries: [
      { file: "devices/README.md", label: "Supported devices" },
      { file: "roadmap.md", label: "Roadmap" },
      { file: "server/running.md", label: "Running the server" },
      { file: "glossary.md", label: "Glossary" },
    ],
  },
  {
    label: "Contribute",
    entries: [{ onGitHub: "development/README.md", label: "Development guide" }],
  },
];

export const isPage = (entry: NavEntry): entry is NavPage => "file" in entry;

/** Every published file, by its path under docs/. */
export const PUBLISHED: readonly string[] = NAV.flatMap((group) =>
  group.entries.filter(isPage).map((entry) => entry.file),
);

/** A sidebar entry's address on GitHub. */
export const onGitHubUrl = (entry: NavOnGitHub) => blob(`docs/${entry.onGitHub}`);
