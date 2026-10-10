<script lang="ts">
  import PaneTree from "./PaneTree.svelte";
  import TerminalView from "./TerminalView.svelte";
  import Icon from "../icons/Icon.svelte";
  import { t } from "../../i18n";
  import type { LayoutNode, SplitDir } from "../../util/layout";
  import { sessions } from "../../stores/sessions.svelte";
  import { toasts } from "../../stores/toasts.svelte";

  let { node, tabId, multi = false }: { node: LayoutNode; tabId: string; multi?: boolean } = $props();

  const DT_TAB = "application/x-aht-tab";
  const DT_CONN = "application/x-aht-connection";

  let el = $state<HTMLDivElement>();
  let zone = $state<"right" | "down" | null>(null);

  function dragSplit(e: PointerEvent) {
    if (node.type !== "split") return;
    e.preventDefault();
    const handle = e.currentTarget as HTMLElement;
    handle.setPointerCapture(e.pointerId);
    const rect = el!.getBoundingClientRect();
    const id = node.id;
    const dir = node.dir;
    const move = (ev: PointerEvent) => {
      const r = dir === "row" ? (ev.clientX - rect.left) / rect.width : (ev.clientY - rect.top) / rect.height;
      sessions.setRatio(tabId, id, r);
    };
    const up = () => {
      handle.removeEventListener("pointermove", move);
      handle.removeEventListener("pointerup", up);
    };
    handle.addEventListener("pointermove", move);
    handle.addEventListener("pointerup", up);
  }

  function splitKey(e: KeyboardEvent) {
    if (node.type !== "split") return;
    const d = node.dir === "row" ? ["ArrowLeft", "ArrowRight"] : ["ArrowUp", "ArrowDown"];
    if (e.key === d[0]) sessions.setRatio(tabId, node.id, node.ratio - 0.05);
    if (e.key === d[1]) sessions.setRatio(tabId, node.id, node.ratio + 0.05);
  }

  function accepts(e: DragEvent) {
    const types = e.dataTransfer?.types ?? [];
    return types.includes(DT_TAB) || types.includes(DT_CONN);
  }
  function over(e: DragEvent) {
    if (!accepts(e) || node.type !== "leaf") return;
    e.preventDefault();
    const r = el!.getBoundingClientRect();
    const fx = (e.clientX - r.left) / r.width;
    const fy = (e.clientY - r.top) / r.height;
    zone = fx - 0.5 > fy - 0.5 ? "right" : "down";
  }
  async function drop(e: DragEvent) {
    if (node.type !== "leaf" || !zone) return;
    e.preventDefault();
    const dir: SplitDir = zone === "right" ? "row" : "col";
    zone = null;
    const tab = e.dataTransfer?.getData(DT_TAB);
    const conn = e.dataTransfer?.getData(DT_CONN);
    if (tab) sessions.mergeTab(tab, tabId, node.id, dir);
    else if (conn) {
      sessions.focus(tabId, node.id);
      await sessions.open(conn, dir).catch(toasts.error);
    }
  }
</script>

{#if node.type === "split"}
  <div class="split {node.dir}" bind:this={el}>
    <div class="child" style:flex-basis="{node.ratio * 100}%"><PaneTree node={node.a} {tabId} multi /></div>
    <div
      class="divider"
      role="separator"
      tabindex="0"
      aria-orientation={node.dir === "row" ? "vertical" : "horizontal"}
      aria-valuenow={Math.round(node.ratio * 100)}
      aria-label={t("common.resize")}
      onpointerdown={dragSplit}
      onkeydown={splitKey}
    ></div>
    <div class="child" style:flex-basis="{(1 - node.ratio) * 100}%"><PaneTree node={node.b} {tabId} multi /></div>
  </div>
{:else}
  {@const info = sessions.infos[node.sessionId]}
  {@const focused = sessions.activeTabId === tabId && sessions.activeTab?.activeLeaf === node.id}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="pane"
    class:focused={multi && focused}
    bind:this={el}
    onpointerdown={() => !focused && sessions.focus(tabId, node.id)}
    ondragover={over}
    ondragleave={(e) => !el!.contains(e.relatedTarget as Node) && (zone = null)}
    ondrop={drop}
  >
    {#if multi}
      <div class="pane-head">
        <span class="dot {info?.state ?? ''}"></span>
        <span class="env-tick env-{info?.env ?? 'none'}"></span>
        <span class="ptitle">{info?.title ?? "…"}</span>
        <button class="icon-btn" title={t("tabs.closePane")} aria-label={t("tabs.closePane")} onclick={() => sessions.closePane(tabId, node.id)}>
          <Icon name="x" size={12} />
        </button>
      </div>
    {/if}
    <div class="pane-body"><TerminalView sessionId={node.sessionId} {focused} /></div>
    {#if zone}
      <div class="zone {zone}" aria-hidden="true">
        <span>{zone === "right" ? t("tabs.dropRight") : t("tabs.dropDown")}</span>
      </div>
    {/if}
  </div>
{/if}

<style>
  .split {
    display: flex;
    width: 100%;
    height: 100%;
    min-width: 0;
    min-height: 0;
  }
  .split.col {
    flex-direction: column;
  }
  .child {
    flex-grow: 0;
    flex-shrink: 1;
    min-width: 0;
    min-height: 0;
    overflow: hidden;
  }
  .divider {
    flex: none;
    background: var(--border);
    position: relative;
    z-index: 2;
    outline: none;
  }
  .row > .divider {
    width: 1px;
    cursor: col-resize;
  }
  .col > .divider {
    height: 1px;
    cursor: row-resize;
  }
  .divider::after {
    content: "";
    position: absolute;
    inset: -4px;
  }
  .divider:hover,
  .divider:focus-visible {
    background: var(--accent-line);
  }
  .pane {
    position: relative;
    display: flex;
    flex-direction: column;
    width: 100%;
    height: 100%;
    min-width: 0;
    min-height: 0;
  }
  .pane-head {
    display: flex;
    align-items: center;
    gap: 7px;
    height: 26px;
    padding: 0 4px 0 10px;
    border-bottom: 1px solid var(--border);
    background: var(--panel);
    font-size: var(--fs-meta);
    color: var(--muted);
  }
  .pane.focused .pane-head {
    color: var(--text);
    box-shadow: inset 0 1px 0 var(--accent);
  }
  .pane-head .icon-btn {
    width: 20px;
    height: 20px;
  }
  .ptitle {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .env-tick {
    width: 3px;
    height: 10px;
    border-radius: 2px;
    display: none;
  }
  .env-tick.env-prod {
    display: block;
    background: var(--env-prod);
  }
  .env-tick.env-staging {
    display: block;
    background: var(--env-staging);
  }
  .env-tick.env-dev {
    display: block;
    background: var(--env-dev);
  }
  .pane-body {
    flex: 1;
    min-height: 0;
  }
  .zone {
    position: absolute;
    z-index: 4;
    display: grid;
    place-items: center;
    border: 1px solid var(--accent-line);
    background: var(--accent-soft);
    border-radius: var(--r);
    color: var(--accent-text);
    font-size: var(--fs-meta);
    pointer-events: none;
    animation: fade var(--t-fast) var(--ease);
  }
  .zone.right {
    top: 4px;
    bottom: 4px;
    right: 4px;
    width: calc(50% - 6px);
  }
  .zone.down {
    left: 4px;
    right: 4px;
    bottom: 4px;
    height: calc(50% - 6px);
  }
  @keyframes fade {
    from {
      opacity: 0;
    }
  }
</style>
