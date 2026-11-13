// The brand files are imported from assets/brand/, their one source, so
// neither site carries a copy that drifts from the other or from the app.
// Each app's Vite build resolves the `?url` imports to hashed assets.
export { default as favicon } from "../../../../../assets/brand/favicon.svg?url";
export { default as lockup } from "../../../../../assets/brand/lockup.svg?url";
export { default as lockupOnLight } from "../../../../../assets/brand/lockup-on-light.svg?url";

import tokens from "../../../../../assets/brand/tokens.json";

/** The browser chrome color: the page's background in each theme. */
export const themeColor = { light: tokens.color.mist, dark: tokens.color.graphite };
