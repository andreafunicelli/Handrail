<script lang="ts">
  import { onDestroy } from "svelte";
  import Icon from "../icons/Icon.svelte";
  import { t } from "../../i18n";
  import { copyText } from "../../util/clipboard";

  /**
   * Copies `text` and says so in place: the icon and label swap to "Copied" (or "Copy failed")
   * for a moment, in the same space, then return. Clicking again restarts the moment instead of
   * stacking. Ported from Interior's copy-button (github.com/ddoemonn/interior, MIT).
   */
  let {
    text,
    label = t("copy.copy"),
    doneLabel = t("copy.done"),
    class: cls = "btn sm",
  }: { text: string; label?: string; doneLabel?: string; class?: string } = $props();

  let status = $state<"idle" | "copied" | "error">("idle");
  let timer: ReturnType<typeof setTimeout> | undefined;

  async function copy() {
    const ok = await copyText(text);
    clearTimeout(timer);
    status = ok ? "copied" : "error";
    timer = setTimeout(() => (status = "idle"), ok ? 1600 : 2400);
  }
  onDestroy(() => clearTimeout(timer));
</script>

<button class="{cls} copy-btn" class:copied={status === "copied"} class:failed={status === "error"} onclick={copy}>
  <span class="face" class:off={status !== "idle"}><Icon name="copy" size={13} />{label}</span>
  <span class="face" class:off={status !== "copied"} aria-hidden={status !== "copied"}><Icon name="check" size={13} />{doneLabel}</span>
  <span class="face" class:off={status !== "error"} aria-hidden={status !== "error"}><Icon name="x" size={13} />{t("copy.failed")}</span>
  <span class="visually-hidden" role="status" aria-live="polite">{status === "copied" ? doneLabel : status === "error" ? t("copy.failed") : ""}</span>
</button>

<style>
  .copy-btn {
    display: inline-grid;
  }
  .face {
    grid-area: 1 / 1;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    white-space: nowrap;
    transition: opacity var(--t) var(--ease), visibility var(--t);
  }
  .face.off {
    opacity: 0;
    visibility: hidden;
  }
  .copied {
    color: var(--ok);
  }
  .failed {
    color: var(--danger);
  }
</style>
