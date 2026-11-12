<script lang="ts">
  import Icon from "../icons/Icon.svelte";
  import { t } from "../../i18n";
  import type { Connection, Group } from "../../api/types";
  import { fuzzyFilter } from "../../util/fuzzy";
  import { groupPath } from "../../util/tree";

  /**
   * The SSH servers a connection goes through, in order (ProxyJump): the chain as an ordered
   * list (move up/down, remove), and a search box with a drop-down to add a server. Replaces a
   * row of chips that became unusable with many servers.
   */
  let {
    candidates,
    groups,
    value = $bindable(),
    label,
  }: { candidates: Connection[]; groups: Group[]; value: string[]; label: string } = $props();

  const LIMIT = 50;
  const uid = `jump-${Math.random().toString(36).slice(2, 8)}`;

  let query = $state("");
  let open = $state(false);
  let active = $state(0);
  let input = $state<HTMLInputElement | null>(null);
  let closing: ReturnType<typeof setTimeout> | undefined;

  function show() {
    clearTimeout(closing);
    open = true;
  }
  // Late, so that a pointer on an option lands first; cancelled if focus comes back.
  function hideSoon() {
    clearTimeout(closing);
    closing = setTimeout(() => (open = false), 120);
  }

  const byId = $derived(new Map(candidates.map((c) => [c.id, c])));
  /** The chain, skipping ids that are no longer SSH connections (still kept in `value`). */
  const chain = $derived(value.map((id) => ({ id, c: byId.get(id) ?? null })));
  const pathOf = (c: Connection) => (c.group_id ? groupPath(groups, c.group_id) : "");
  const matches = $derived(
    fuzzyFilter(
      candidates.filter((c) => !value.includes(c.id)),
      query,
      (c) => [c.name, c.host, c.user ? `${c.user}@${c.host}` : "", pathOf(c), ...c.tags],
    ),
  );
  const shown = $derived(matches.slice(0, LIMIT));

  $effect(() => {
    // Keep the highlighted row inside the list as it shrinks.
    if (active >= shown.length) active = Math.max(0, shown.length - 1);
  });

  function add(c: Connection) {
    value = [...value, c.id];
    query = "";
    active = 0;
    input?.focus();
  }
  function remove(id: string) {
    value = value.filter((x) => x !== id);
  }
  function move(i: number, d: -1 | 1) {
    const j = i + d;
    if (j < 0 || j >= value.length) return;
    const next = [...value];
    [next[i], next[j]] = [next[j], next[i]];
    value = next;
  }

  function onkeydown(e: KeyboardEvent) {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      show();
      active = Math.min(active + 1, shown.length - 1);
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      active = Math.max(active - 1, 0);
    } else if (e.key === "Enter") {
      // Never submit the editor form from here.
      e.preventDefault();
      if (open && shown[active]) add(shown[active]);
    } else if (e.key === "Escape" && open) {
      // Close the list, not the editor.
      e.preventDefault();
      e.stopPropagation();
      open = false;
    }
  }

  const who = (c: Connection) => (c.user ? `${c.user}@${c.host}` : c.host);
</script>

<div class="picker">
  {#if chain.length}
    <ol class="chain" aria-label={label}>
      {#each chain as hop, i (hop.id)}
        <li class="hop">
          <span class="ord" aria-hidden="true">{i + 1}</span>
          {#if hop.c}
            <span class="env {hop.c.env}" aria-hidden="true"></span>
            <span class="name">{hop.c.name}</span>
            {#if hop.c.env === "prod"}<span class="env-prod-badge">PROD</span>{/if}
            <span class="who mono">{who(hop.c)}</span>
          {:else}
            <span class="name missing">{t("editor.jumpMissing")}</span>
          {/if}
          <span class="grow"></span>
          <button type="button" class="icon-btn" disabled={i === 0} title={t("editor.jumpUp")} aria-label={t("editor.jumpUp")} onclick={() => move(i, -1)}
            ><Icon name="arrowUp" size={12} /></button
          >
          <button type="button" class="icon-btn" disabled={i === chain.length - 1} title={t("editor.jumpDown")} aria-label={t("editor.jumpDown")} onclick={() => move(i, 1)}
            ><Icon name="arrowDown" size={12} /></button
          >
          <button type="button" class="icon-btn" title={t("editor.jumpRemove")} aria-label={t("editor.jumpRemove")} onclick={() => remove(hop.id)}
            ><Icon name="x" size={12} /></button
          >
        </li>
      {/each}
    </ol>
  {:else}
    <p class="hint">{t("editor.jumpNone")}</p>
  {/if}

  <div class="combo">
    <Icon name="search" size={13} />
    <input
      bind:this={input}
      class="input"
      type="text"
      role="combobox"
      aria-label={t("editor.jumpAdd")}
      aria-expanded={open}
      aria-controls="{uid}-list"
      aria-autocomplete="list"
      aria-activedescendant={open && shown[active] ? `${uid}-${shown[active].id}` : undefined}
      placeholder={t(chain.length ? "editor.jumpAddNext" : "editor.jumpAdd")}
      spellcheck="false"
      autocomplete="off"
      bind:value={query}
      oninput={() => (show(), (active = 0))}
      onfocus={show}
      onblur={hideSoon}
      {onkeydown}
    />
  </div>
  {#if open}
    <ul class="list" id="{uid}-list" role="listbox" aria-label={t("editor.jumpAdd")}>
      {#each shown as c, i (c.id)}
        <!-- Pointer down, not click: the input's blur would close the list first. -->
        <li
          id="{uid}-{c.id}"
          role="option"
          aria-selected={i === active}
          class:active={i === active}
          onpointerdown={(e) => (e.preventDefault(), add(c))}
          onpointermove={() => (active = i)}
        >
          <span class="env {c.env}" aria-hidden="true"></span>
          <span class="name">{c.name}</span>
          {#if c.env === "prod"}<span class="env-prod-badge">PROD</span>{/if}
          <span class="who mono">{who(c)}</span>
          {#if pathOf(c)}<span class="path">{pathOf(c)}</span>{/if}
        </li>
      {:else}
        <li class="empty" role="option" aria-selected="false" aria-disabled="true">
          {t(candidates.length > value.length ? "editor.jumpNoMatch" : "editor.jumpAllUsed")}
        </li>
      {/each}
      {#if matches.length > LIMIT}
        <li class="more" role="option" aria-selected="false" aria-disabled="true">{t("editor.jumpMore", { n: matches.length - LIMIT })}</li>
      {/if}
    </ul>
  {/if}
</div>

<style>
  .picker {
    display: flex;
    flex-direction: column;
    gap: 6px;
    position: relative;
  }
  .chain {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .hop {
    display: flex;
    align-items: center;
    gap: 8px;
    min-height: var(--row-h);
    padding: 0 4px 0 8px;
    border: 1px solid var(--border);
    border-radius: var(--r);
    background: var(--panel);
  }
  .ord {
    display: inline-grid;
    place-items: center;
    width: 18px;
    height: 18px;
    border-radius: 999px;
    background: var(--accent-soft);
    color: var(--accent-text);
    font-size: var(--fs-tiny);
    font-weight: 600;
    flex: none;
  }
  .env {
    width: 3px;
    height: 14px;
    border-radius: 2px;
    background: var(--border-strong);
    flex: none;
  }
  .env.prod {
    background: var(--env-prod);
  }
  .env.staging {
    background: var(--env-staging);
  }
  .env.dev {
    background: var(--env-dev);
  }
  .name {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .missing {
    color: var(--muted);
    font-style: italic;
  }
  .who {
    color: var(--muted);
    font-size: var(--fs-small);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .path {
    margin-left: auto;
    color: var(--faint);
    font-size: var(--fs-small);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 40%;
  }
  .grow {
    flex: 1;
  }
  .combo {
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--muted);
  }
  .combo .input {
    flex: 1;
  }
  .list {
    list-style: none;
    margin: 0;
    padding: 4px;
    max-height: 240px;
    overflow: auto;
    border-radius: var(--r);
    background: var(--raised);
    box-shadow: var(--shadow-float);
  }
  .list li {
    display: flex;
    align-items: center;
    gap: 8px;
    min-height: var(--row-h);
    padding: 0 8px;
    border-radius: var(--r-sm);
    cursor: pointer;
  }
  .list li.active {
    background: var(--hover);
  }
  .list li.empty,
  .list li.more {
    color: var(--muted);
    cursor: default;
  }
  .hint {
    margin: 0;
    color: var(--muted);
    font-size: var(--fs-small);
  }
</style>
