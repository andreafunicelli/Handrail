<script lang="ts">
  import Icon, { type IconName } from "../icons/Icon.svelte";
  import Modal from "../common/Modal.svelte";
  import { t, type MessageKey } from "../../i18n";
  import { api } from "../../api";
  import type { ApprovalMode } from "../../api/types";
  import { fuzzyFilter } from "../../util/fuzzy";
  import { groupPath } from "../../util/tree";
  import { needsTypedConfirm } from "../../util/approval";
  import { connections, blankConnection } from "../../stores/connections.svelte";
  import { sessions } from "../../stores/sessions.svelte";
  import { agent } from "../../stores/agent.svelte";
  import { settings } from "../../stores/settings.svelte";
  import { toasts } from "../../stores/toasts.svelte";
  import { ui, type SettingsPage } from "../../stores/ui.svelte";
  import { vault } from "../../stores/vault.svelte";

  interface Action {
    id: string;
    label: string;
    hint?: string;
    icon: IconName;
    group: "connections" | "actions";
    run: () => unknown;
  }

  let query = $state("");
  let index = $state(0);
  let list = $state<HTMLDivElement>();

  const pages: SettingsPage[] = ["providers", "harnesses", "approvals", "import", "sync", "appearance", "vault", "about"];

  const actions = $derived.by<Action[]>(() => {
    const out: Action[] = [];
    for (const c of connections.list)
      out.push({
        id: "c:" + c.id,
        label: t("palette.connect", { name: c.name }),
        hint: groupPath(connections.groups, c.group_id) || `${c.user}@${c.host}`,
        icon: "server",
        group: "connections",
        run: () => sessions.open(c.id),
      });
    out.push(
      { id: "local", label: t("palette.newLocal"), icon: "terminal", group: "actions", run: () => sessions.openLocal() },
      { id: "newconn", label: t("palette.newConnection"), icon: "plus", group: "actions", run: () => ui.edit(blankConnection(), true) },
      { id: "ai", label: t("palette.toggleAi"), icon: "sparkle", group: "actions", hint: "Ctrl+J", run: () => (settings.aiOpen = !settings.aiOpen) },
    );
    const sid = sessions.activeSessionId;
    for (const o of agent.options)
      out.push({ id: "b:" + o.key, label: t("palette.brain", { name: o.label }), hint: o.detail, icon: "sparkle", group: "actions", run: () => agent.setBrain(sid, o.key) });
    const s = sessions.activeSession;
    if (s)
      for (const m of ["confirm_all", "confirm_writes", "auto"] as ApprovalMode[]) {
        if (needsTypedConfirm(s.env, m)) continue; // prod Auto must go through the typed confirmation in the sidebar
        out.push({
          id: "m:" + m,
          label: t("palette.mode", { mode: t(`ai.mode.${m}`) }),
          hint: s.title,
          icon: "shield",
          group: "actions",
          run: async () => {
            await api.setApprovalMode(s.session_id, m);
            sessions.infos[s.session_id] = { ...s, approval_mode: m };
          },
        });
      }
    for (const w of sessions.workspaces)
      out.push({ id: "w:" + w.id, label: t("palette.workspace", { name: w.name }), icon: "layout", group: "actions", run: () => sessions.openWorkspace(w.id) });
    for (const p of pages)
      out.push({ id: "s:" + p, label: t("palette.settings", { page: t(`settings.${p}` as MessageKey) }), icon: "settings", group: "actions", run: () => ui.openSettings(p) });
    out.push({ id: "lock", label: t("palette.lock"), icon: "lock", group: "actions", run: () => vault.lock() });
    return out;
  });

  const results = $derived(
    query.trim()
      ? fuzzyFilter(actions, query, (a) => [a.label, a.hint ?? ""]).slice(0, 50)
      : [...actions.filter((a) => a.group === "actions").slice(0, 3), ...actions.filter((a) => a.group === "connections").slice(0, 8), ...actions.filter((a) => a.group === "actions").slice(3)],
  );

  $effect(() => {
    void query;
    index = 0;
  });

  function close() {
    ui.paletteOpen = false;
  }

  async function run(a: Action | undefined) {
    if (!a) return;
    close();
    try {
      await a.run();
    } catch (e) {
      toasts.error(e);
    }
  }

  function key(e: KeyboardEvent) {
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      index = (index + (e.key === "ArrowDown" ? 1 : -1) + results.length) % Math.max(1, results.length);
      requestAnimationFrame(() => list?.querySelector(`[data-i="${index}"]`)?.scrollIntoView({ block: "nearest" }));
    } else if (e.key === "Enter") {
      e.preventDefault();
      void run(results[index]);
    }
  }
</script>

<Modal label={t("palette.placeholder")} onclose={close} width="600px">
  <div class="palette">
    <div class="q">
      <Icon name="search" size={15} />
      <input bind:value={query} placeholder={t("palette.placeholder")} onkeydown={key} aria-label={t("palette.placeholder")} aria-controls="palette-list" />
      <span class="kbd">Esc</span>
    </div>
    <div class="list" id="palette-list" role="listbox" bind:this={list}>
      {#each results as a, i (a.id)}
        {#if i === 0 || results[i - 1].group !== a.group}
          <div class="grp">{t(`palette.group.${a.group}`)}</div>
        {/if}
        <button role="option" aria-selected={i === index} class:on={i === index} data-i={i} onmousemove={() => (index = i)} onclick={() => run(a)}>
          <Icon name={a.icon} size={14} />
          <span class="lbl">{a.label}</span>
          {#if a.hint}<span class="hint">{a.hint}</span>{/if}
        </button>
      {:else}
        <p class="none">{t("palette.empty")}</p>
      {/each}
    </div>
  </div>
</Modal>

<style>
  .palette {
    display: flex;
    flex-direction: column;
    max-height: min(520px, calc(100vh - 80px));
  }
  .q {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 0 14px;
    height: 48px;
    border-bottom: 1px solid var(--border);
    color: var(--muted);
  }
  .q input {
    flex: 1;
    min-width: 0;
    border: 0;
    outline: 0;
    background: none;
    font-size: 14px;
    color: var(--text);
  }
  .q input::placeholder {
    color: var(--faint);
  }
  .list {
    overflow: auto;
    padding: 4px 6px 8px;
  }
  .grp {
    padding: 8px 8px 4px;
    font-size: var(--fs-small);
    color: var(--faint);
  }
  button {
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    height: 32px;
    padding: 0 10px;
    border-radius: var(--r);
    text-align: left;
    color: var(--text-2);
  }
  button :global(svg) {
    color: var(--faint);
    flex: none;
  }
  button.on {
    background: var(--active);
    color: var(--text);
  }
  button.on :global(svg) {
    color: var(--accent-text);
  }
  .lbl {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .hint {
    font-size: var(--fs-small);
    color: var(--faint);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 40%;
  }
  .none {
    margin: 0;
    padding: 18px 10px;
    color: var(--faint);
  }
</style>
