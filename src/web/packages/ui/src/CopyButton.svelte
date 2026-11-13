<script lang="ts">
  let { text, label = "Copy" }: { text: string; label?: string } = $props();

  // The docs site is served over HTTPS only, where the clipboard API exists;
  // a refusal (a denied permission) says so rather than claim a copy.
  let shown = $state<"idle" | "copied" | "failed">("idle");
  let timer: ReturnType<typeof setTimeout> | undefined;

  async function copy() {
    try {
      await navigator.clipboard.writeText(text);
      shown = "copied";
    } catch {
      shown = "failed";
    }
    clearTimeout(timer);
    timer = setTimeout(() => (shown = "idle"), 1400);
  }
</script>

<button type="button" class="copy" onclick={copy} aria-live="polite">
  {shown === "copied" ? "Copied" : shown === "failed" ? "Copy failed" : label}
</button>

<style>
  .copy {
    height: 26px;
    padding: 0 12px;
    border-radius: var(--radius-pill);
    border: 1px solid var(--color-line-strong);
    background: var(--color-surface);
    font-size: var(--text-xs);
    font-weight: 500;
    cursor: pointer;
    white-space: nowrap;
  }
  .copy:hover {
    border-color: var(--color-text);
  }
  /* A thumb-sized target on phones; the desktop keeps the compact pill. */
  @media (max-width: 760px), (pointer: coarse) {
    .copy {
      height: 44px;
      padding: 0 var(--space-4);
      font-size: var(--text-sm);
    }
  }
</style>
