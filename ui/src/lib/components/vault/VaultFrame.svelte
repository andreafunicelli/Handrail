<script lang="ts">
  import Logo from "../brand/Logo.svelte";
  import type { Snippet } from "svelte";
  import { api } from "../../api";
  import { t } from "../../i18n";
  let { children }: { children: Snippet } = $props();
</script>

<div class="frame">
  <div class="col">
    <div class="brand" aria-hidden="true">
      <span class="mark" aria-hidden="true"><Logo size={16} /></span>
      <span class="name">{t("app.name")}</span>
    </div>
    {@render children()}
    {#if api.backend === "mock"}
      <p class="demo">{t("vault.mockNote")}</p>
    {/if}
  </div>
</div>

<style>
  .frame {
    height: 100%;
    overflow: auto;
    display: grid;
    place-items: center;
    padding: 32px 16px;
  }
  .col {
    width: min(400px, 100%);
    animation: rise 220ms var(--ease);
  }
  .brand {
    display: flex;
    align-items: center;
    gap: 9px;
    margin-bottom: 36px;
    color: var(--text-2);
    font-weight: 550;
    letter-spacing: -0.01em;
  }
  .mark {
    display: grid;
    place-items: center;
    width: 26px;
    height: 26px;
    border-radius: 7px;
    border: 1px solid var(--border-strong);
    color: var(--text);
    font-size: 12px;
  }
  .demo {
    margin-top: 28px;
    font-size: var(--fs-meta);
    color: var(--faint);
  }
  .col :global(h1) {
    margin: 0 0 6px;
    font-size: 20px;
    font-weight: 600;
    letter-spacing: -0.015em;
  }
  .col :global(.lede) {
    margin: 0 0 24px;
    color: var(--muted);
    max-width: 52ch;
  }
  @keyframes rise {
    from {
      opacity: 0;
      transform: translateY(4px);
    }
  }
</style>
