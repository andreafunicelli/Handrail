<script lang="ts">
  import { renderMarkdown } from "../../util/markdown";
  let { source }: { source: string } = $props();
  // Safe: renderMarkdown escapes every piece of source text and emits only allow-listed,
  // attribute-free tags (see util/markdown.ts and its tests).
  const html = $derived(renderMarkdown(source));
</script>

<div class="md">{@html html}</div>

<style>
  .md {
    overflow-wrap: anywhere;
    color: var(--text-2);
  }
  .md :global(p) {
    margin: 0 0 8px;
  }
  .md :global(p:last-child),
  .md :global(ul:last-child),
  .md :global(ol:last-child),
  .md :global(pre:last-child) {
    margin-bottom: 0;
  }
  .md :global(strong) {
    color: var(--text);
    font-weight: 600;
  }
  .md :global(h3),
  .md :global(h4),
  .md :global(h5),
  .md :global(h6) {
    margin: 10px 0 6px;
    font-size: var(--fs);
    font-weight: 600;
    color: var(--text);
  }
  .md :global(ul),
  .md :global(ol) {
    margin: 0 0 8px;
    padding-left: 18px;
  }
  .md :global(li) {
    margin: 2px 0;
  }
  .md :global(li::marker) {
    color: var(--faint);
  }
  .md :global(code) {
    font-family: var(--font-mono);
    font-size: 11.5px;
    padding: 1px 4px;
    border-radius: var(--r-sm);
    background: var(--hover);
    color: var(--text);
  }
  .md :global(pre) {
    margin: 0 0 8px;
    padding: 9px 11px;
    border-radius: var(--r);
    background: var(--bg);
    border: 1px solid var(--border);
    overflow-x: auto;
  }
  .md :global(pre code) {
    padding: 0;
    background: none;
    font-size: 11.5px;
    line-height: 1.55;
    white-space: pre;
  }
</style>
