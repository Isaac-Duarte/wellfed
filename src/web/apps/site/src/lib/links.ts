// Every outbound link in one place, so moving the repository or a doc is one edit.
export const SITE = "https://well-fed.dev";
export const REPO = "https://github.com/Isaac-Duarte/wellfed";
export const DISCORD = "https://discord.gg/5UfwjAsnw9";

const doc = (path: string) => `${REPO}/blob/master/${path}`;

export const DOCS = {
  roadmap: doc("docs/roadmap.md"),
  license: doc("LICENSE"),
  assets: doc("assets/README.md#license"),
} as const;
