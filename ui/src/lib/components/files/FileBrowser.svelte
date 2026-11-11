<script lang="ts">
  import { onMount } from "svelte";
  import Icon from "../icons/Icon.svelte";
  import FilePane from "./FilePane.svelte";
  import CompareHost from "./CompareHost.svelte";
  import { PaneHandle } from "./paneHandle.svelte";
  import { copyItems } from "./copyFlow";
  import { t, type MessageKey } from "../../i18n";
  import { errorMessage } from "../../api/errors";
  import { s3 } from "../../stores/s3.svelte";
  import { sessions } from "../../stores/sessions.svelte";
  import { toasts } from "../../stores/toasts.svelte";
  import { placeLabel } from "../../stores/transfers.svelte";
  import { localBackend, s3Backend, sftpBackend, type FileBackend } from "../../util/fileBackends";
  import { copyBlock, moveBlock, type CopyBlock, type MoveBlock } from "../../util/fileOps";
  import { joinPath } from "../../util/files";

  /**
   * The file browser of one remote file system, optionally with a second pane next to it on
   * this computer, another (or the same) SSH session, or an S3 account: copy and move between
   * the two.
   */
  let { fs, label }: { fs: FileBackend; label: string } = $props();

  let dual = $state(false);
  /** `FileBackend.key` of the second pane's file system. */
  let otherKey = $state("");
  let activeSide = $state<0 | 1>(0);
  const panes = [new PaneHandle(), new PaneHandle()];

  interface Target {
    key: string;
    label: string;
    make: () => FileBackend;
  }

  /** Where the second pane can be: this computer, every connected SSH session, every S3 account. */
  const targets = $derived.by(() => {
    const out: Target[] = [{ key: "local", label: t("files.compare.thisComputer"), make: localBackend }];
    for (const i of Object.values(sessions.infos)) {
      if (!i.connection_id || i.state === "disconnected") continue;
      const key = `sftp:${i.session_id}`;
      const here = key === fs.key ? ` ${t("files.copySame")}` : "";
      out.push({ key, label: `${i.title} — ${i.user ?? ""}@${i.host ?? ""}${here}`, make: () => sftpBackend(i.session_id) });
    }
    for (const a of s3.sorted) out.push({ key: `s3:${a.id}`, label: `${a.name} (S3)`, make: () => s3Backend(a.id) });
    return out;
  });

  // One backend per destination, so the second pane is not rebuilt when the session list changes.
  const made = new Map<string, FileBackend>();
  const other = $derived.by(() => {
    const x = targets.find((x) => x.key === otherKey);
    if (!x) return null;
    if (!made.has(x.key)) made.set(x.key, x.make());
    return { fs: made.get(x.key)!, label: x.label };
  });

  function toggleDual() {
    dual = !dual;
    activeSide = 0;
    // This computer first: copying to and from it is what the second pane is mostly for.
    if (dual && !targets.some((x) => x.key === otherKey)) otherKey = targets[0]?.key ?? "";
  }

  onMount(() => {
    if (!s3.list.length) void s3.load().catch(() => {});
  });

  // ---- operations between the panes ----
  const src = $derived(panes[activeSide]);
  const dst = $derived(panes[activeSide === 0 ? 1 : 0]);
  const ready = $derived(dual && !!other && !!src.fs && !!dst.fs);
  const cBlock = $derived<CopyBlock | null>(ready ? copyBlock(src.fs!, src.picked, dst.fs!, dst.path) : "none");
  /** Moving is a rename, so it is offered only when both panes are on the same file system. */
  const samePlace = $derived(ready && src.fs!.key === dst.fs!.key);
  const mBlock = $derived<MoveBlock | null>(samePlace ? moveBlock(src.fs!, src.picked, dst.fs!, dst.path) : "otherPlace");

  const COPY_WHY: Record<CopyBlock, MessageKey> = {
    none: "files.dual.pickToCopy",
    s3ToS3: "files.dual.noS3ToS3",
    folderS3: "files.dual.noFolderS3",
    folderLocal: "files.dual.noFolderLocal",
    notWritable: "files.dual.notWritable",
    intoItself: "files.dual.intoItself",
  };
  const MOVE_WHY: Record<MoveBlock, MessageKey> = {
    none: "files.dual.pickToCopy",
    otherPlace: "files.dual.noMoveOther",
    otherBucket: "files.dual.noMoveBucket",
    s3Folder: "files.dual.noMoveS3Folder",
    notWritable: "files.dual.notWritable",
    sameFolder: "files.dual.sameFolder",
    intoItself: "files.dual.intoItself",
  };
  /** An unsupported pair is explained in the bar, not only in a tooltip. */
  const pairNote = $derived(ready && src.fs!.kind === "s3" && dst.fs!.kind === "s3" ? t("files.dual.noS3ToS3") : "");

  const why = $derived(pairNote || (cBlock && cBlock !== "none" ? t(COPY_WHY[cBlock]) : ""));

  async function copyAcross() {
    if (cBlock) return void toasts.info(t(COPY_WHY[cBlock]));
    const [from, to, dir, items] = [src.fs!, dst.fs!, dst.path, [...src.picked]];
    const { copied, last } = await copyItems(from, items, to, dir);
    const where = placeLabel(to);
    if (copied === 1 && items.length === 1) toasts.success(t("files.copied", { path: last, where }));
    else if (copied) toasts.success(t("files.copiedMany", { n: copied, where }));
  }

  async function moveAcross() {
    if (mBlock) return void toasts.info(t(MOVE_WHY[mBlock]));
    const [from, to, dir, items] = [src.fs!, dst, dst.path, [...src.picked]];
    const taken = new Set(to.entries.map((e) => e.name));
    let moved = 0;
    for (const e of items) {
      // A move never replaces anything: rename would overwrite without asking.
      if (taken.has(e.name)) {
        toasts.error(t("files.dual.moveExists", { name: e.name }));
        continue;
      }
      try {
        await from.rename(e, joinPath(dir, e.name));
        moved++;
      } catch (err) {
        toasts.error(items.length > 1 ? `${e.name}: ${errorMessage(err)}` : err);
      }
    }
    await Promise.all(panes.map((p) => p.refresh().catch(() => {})));
    if (moved) toasts.success(t("files.dual.moved", { n: moved, path: dir }));
  }

  function onkeydown(ev: KeyboardEvent) {
    if (!dual) return;
    const el = ev.target instanceof HTMLElement ? ev.target : null;
    const typing = !!el && (el.isContentEditable || ["INPUT", "TEXTAREA", "SELECT"].includes(el.tagName));
    if (ev.key === "F5" || ev.key === "F6") {
      // F5 would reload the window.
      ev.preventDefault();
      if (typing || ev.ctrlKey || ev.metaKey || ev.altKey) return;
      void (ev.key === "F5" ? copyAcross() : samePlace ? moveAcross() : toasts.info(t("files.dual.noMoveOther")));
    } else if (ev.key === "Tab" && !ev.shiftKey && !ev.ctrlKey && !ev.altKey && el?.closest(".list")) {
      // From a list, Tab goes to the other pane's list; Shift+Tab still leaves the list.
      ev.preventDefault();
      activeSide = activeSide === 0 ? 1 : 0;
      panes[activeSide].focus();
    }
  }

  const count = $derived(src.picked.length);
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="fb" {onkeydown}>
  {#if dual}
    <div class="dualbar" role="toolbar" aria-label={t("files.dual.toolbar")}>
      <label class="dest">
        <span class="lbl">{t("files.dual.otherPane")}</span>
        <select class="select sm" bind:value={otherKey} aria-label={t("files.dual.otherPane")}>
          {#each targets as x (x.key)}<option value={x.key}>{x.label}</option>{/each}
        </select>
      </label>
      <button
        class="btn sm"
        disabled={!!cBlock}
        title={cBlock ? t(COPY_WHY[cBlock]) : t("files.dual.copyTip")}
        aria-label={count ? t("files.dual.copyN", { n: count }) : t("files.dual.copy")}
        aria-keyshortcuts="F5"
        onclick={copyAcross}
      >
        <Icon name="copy" size={13} /><span>{count ? t("files.dual.copyN", { n: count }) : t("files.dual.copy")}</span>{@render dirArrow()}
      </button>
      {#if samePlace}
        <button
          class="btn sm"
          disabled={!!mBlock}
          title={mBlock ? t(MOVE_WHY[mBlock]) : t("files.dual.moveTip")}
          aria-label={count ? t("files.dual.moveN", { n: count }) : t("files.dual.move")}
          aria-keyshortcuts="F6"
          onclick={moveAcross}
        >
          <Icon name="send" size={13} /><span>{count ? t("files.dual.moveN", { n: count }) : t("files.dual.move")}</span>{@render dirArrow()}
        </button>
      {/if}
      {#if why}<span class="why" title={why}>{why}</span>{/if}
      <span class="grow"></span>
      <span class="keys">{t(samePlace ? "files.dual.keysMove" : "files.dual.keys")}</span>
      <button class="icon-btn close" title={t("files.dual.off")} aria-label={t("files.dual.off")} onclick={toggleDual}><Icon name="x" size={13} /></button>
    </div>
    {#if other?.fs.kind === "local"}<p class="local"><Icon name="info" size={12} />{t("files.dual.localNote")}</p>{/if}
  {/if}
  <div class="panes" class:dual>
    <FilePane {fs} {label} handle={panes[0]} {dual} active={dual && activeSide === 0} onactivate={() => (activeSide = 0)} ondualtoggle={toggleDual} />
    {#if dual}
      {#if other}
        {#key otherKey}
          <FilePane fs={other.fs} label={other.label} handle={panes[1]} dual active={activeSide === 1} onactivate={() => (activeSide = 1)} />
        {/key}
      {:else}
        <div class="nowhere">{t("files.copyNoTarget")}</div>
      {/if}
    {/if}
  </div>
</div>
<CompareHost rank={1} />

<!-- Toward the other pane: sideways next to each other, up or down when stacked. -->
{#snippet dirArrow()}
  <span class="arrow h" aria-hidden="true">{activeSide === 0 ? "→" : "←"}</span><span class="arrow v" aria-hidden="true">{activeSide === 0 ? "↓" : "↑"}</span>
{/snippet}

<style>
  .fb {
    container-type: inline-size;
    display: flex;
    flex-direction: column;
    width: 100%;
    height: 100%;
    min-width: 0;
    background: var(--bg);
  }
  /* One line whatever is selected: a bar that grew when a row was picked moved the rows under
     the pointer, so a double-click opened the row above. */
  .dualbar {
    display: flex;
    align-items: center;
    flex-wrap: nowrap;
    overflow: hidden;
    gap: var(--sp-2);
    padding: 6px 8px;
    border-bottom: 1px solid var(--border);
    font-size: var(--fs-small);
  }
  .dest {
    display: flex;
    align-items: center;
    gap: 6px;
    flex: 0 1 auto;
    min-width: 80px;
  }
  .dest .lbl {
    color: var(--muted);
    white-space: nowrap;
  }
  .dest select {
    min-width: 0;
    max-width: 260px;
    height: var(--ctl-h-sm);
  }
  .dualbar > :global(.btn) {
    flex: none;
  }
  .why {
    flex: 1 1 0;
    color: var(--warn);
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .grow {
    flex: 1;
  }
  .close {
    flex: none;
  }
  .keys {
    color: var(--faint);
    white-space: nowrap;
  }
  .local {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 0;
    padding: 4px 10px;
    border-bottom: 1px solid var(--border);
    color: var(--faint);
    font-size: var(--fs-small);
  }
  .panes {
    display: flex;
    flex: 1;
    min-height: 0;
  }
  .panes > :global(*) {
    flex: 1 1 0;
    min-width: 0;
  }
  .panes.dual > :global(* + *) {
    border-left: 1px solid var(--border);
  }
  .nowhere {
    padding: 20px;
    color: var(--muted);
    font-size: var(--fs-meta);
  }
  /* Narrow: the panes stack, and the shortcut list goes. */
  @container (max-width: 720px) {
    .panes.dual {
      flex-direction: column;
    }
    .panes.dual > :global(* + *) {
      border-left: 0;
      border-top: 1px solid var(--border);
    }
    .keys,
    .arrow.h {
      display: none;
    }
    .arrow.v {
      display: inline;
    }
  }
  .arrow.v {
    display: none;
  }
</style>
