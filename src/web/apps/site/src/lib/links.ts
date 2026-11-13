// Every outbound link the landing page adds to @wellfed/ui's, so moving a doc is one edit.
import { REPO } from "@wellfed/ui";

const doc = (path: string) => `${REPO}/blob/master/${path}`;

export const DOCS = {
  roadmap: doc("docs/roadmap.md"),
  license: doc("LICENSE"),
  assets: doc("assets/README.md#license"),
} as const;
