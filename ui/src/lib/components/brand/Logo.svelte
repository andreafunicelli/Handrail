<script lang="ts">
  /** The Handrail mark: a rail on two posts over a prompt. Rail in currentColor, prompt in the accent. */
  let { size = 20 }: { size?: number } = $props();
</script>

<svg width={size} height={size} viewBox="190 246 644 532" aria-hidden="true">
  <g fill="currentColor">
    <rect x="206" y="262" width="612" height="72" rx="36" />
    <rect x="270" y="298" width="64" height="464" rx="32" />
    <rect x="690" y="298" width="64" height="464" rx="32" />
  </g>
  <g fill="none" stroke="var(--accent)" stroke-width="56" stroke-linecap="round" stroke-linejoin="round">
    <polyline points="420,474 506,546 420,618" />
    <line x1="552" y1="618" x2="616" y2="618" />
  </g>
</svg>
