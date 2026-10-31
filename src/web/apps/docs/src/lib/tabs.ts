// App | CLI tabs, on a page that writes them with `<!-- tabs -->` (content.ts).
// docs.css hides a panel only under the root's data-tab, which HEAD_SCRIPT sets
// before first paint, so a reader who chose CLI never sees App flash first.
// Without JavaScript it is never set and both panels show. The panels are in the
// rendered Markdown, so they are reached by delegation rather than bound. The
// choice holds across pages because the root outlives a navigation.
// vite.config.ts hashes HEAD_SCRIPT into the page's CSP.
export type Tab = "app" | "cli";

const KEY = "wellfed-docs-tab";

export const HEAD_SCRIPT = `document.documentElement.dataset.tab="app";try{if(localStorage.getItem("${KEY}")==="cli")document.documentElement.dataset.tab="cli"}catch{}`;

/** HEAD_SCRIPT as the element the layout writes into <head>. */
export const HEAD_HTML = `<script>${HEAD_SCRIPT}</script>`;

function isTab(value: string | undefined): value is Tab {
  return value === "app" || value === "cli";
}

function currentTab(): Tab {
  return document.documentElement.dataset.tab === "cli" ? "cli" : "app";
}

function markSelected(): void {
  const tab = currentTab();
  for (const button of document.querySelectorAll<HTMLElement>(".tab")) {
    const selected = button.dataset.tab === tab;
    button.setAttribute("aria-selected", String(selected));
    button.tabIndex = selected ? 0 : -1;
  }
  // A panel takes focus so a keyboard reader can scroll one with no link in it.
  // It is set here because without JavaScript there are no tabs to move from.
  for (const panel of document.querySelectorAll<HTMLElement>(".tab-panel")) panel.tabIndex = 0;
}

function choose(tab: Tab): void {
  document.documentElement.dataset.tab = tab;
  markSelected();
  try {
    localStorage.setItem(KEY, tab);
  } catch {
    // Storage blocked (a private window, cleared site data): the choice lasts this visit.
  }
}

/**
 * Brings the page's tabs in line with the choice. A link into the other panel
 * (a search result, a shared URL) opens it for this visit without changing what
 * is remembered, since following a link isn't choosing.
 */
export function syncTabs(): void {
  const id = decodeURIComponent(location.hash.slice(1));
  const target = id === "" ? null : document.getElementById(id);
  const wanted = target?.closest<HTMLElement>(".tab-panel")?.dataset.tabPanel;
  if (isTab(wanted) && wanted !== currentTab()) {
    document.documentElement.dataset.tab = wanted;
    target?.scrollIntoView();
  }
  markSelected();
}

function tabOf(event: Event): HTMLElement | null {
  return event.target instanceof Element ? event.target.closest<HTMLElement>(".tab") : null;
}

function onclick(event: MouseEvent): void {
  const tab = tabOf(event)?.dataset.tab;
  if (isTab(tab)) choose(tab);
}

// The WAI-ARIA tabs pattern with automatic activation: the arrow keys move
// between tabs and select as they go, Home and End go to either end.
function onkeydown(event: KeyboardEvent): void {
  const button = tabOf(event);
  const from = button?.dataset.tab;
  if (button === null || !isTab(from)) return;

  const other: Tab = from === "app" ? "cli" : "app";
  const next: Tab | undefined =
    event.key === "ArrowLeft" || event.key === "ArrowRight"
      ? other
      : event.key === "Home"
        ? "app"
        : event.key === "End"
          ? "cli"
          : undefined;
  if (next === undefined) return;

  event.preventDefault();
  choose(next);
  button.parentElement?.querySelector<HTMLElement>(`.tab[data-tab="${next}"]`)?.focus();
}

/** Starts the tabs on this document, and returns how to stop them. */
export function mountTabs(): () => void {
  document.addEventListener("click", onclick);
  document.addEventListener("keydown", onkeydown);
  window.addEventListener("hashchange", syncTabs);
  return () => {
    document.removeEventListener("click", onclick);
    document.removeEventListener("keydown", onkeydown);
    window.removeEventListener("hashchange", syncTabs);
  };
}
