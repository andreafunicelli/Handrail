<script lang="ts">
  import { onMount, untrack } from "svelte";
  import Icon from "../icons/Icon.svelte";
  import { t } from "../../i18n";
  import { agent } from "../../stores/agent.svelte";
  import { editors } from "../../stores/editors.svelte";
  import { settings } from "../../stores/settings.svelte";
  import { sudo } from "../../stores/sudo.svelte";
  import { toasts } from "../../stores/toasts.svelte";
  import { ui } from "../../stores/ui.svelte";
  import { editorQuestion } from "../../util/editorAsk";
  import type { EditorHandle } from "../../util/codemirror";

  /** The editor for doc `docId` (see the editors store). */
  let { docId, focused = false }: { docId: string; focused?: boolean } = $props();

  const doc = $derived(editors.docs[docId]);
  const dirty = $derived(editors.dirty(docId));
  const pending = $derived(!!doc?.proposal && !doc.proposal.accepted);

  let host = $state<HTMLDivElement>();
  let diffHost = $state<HTMLDivElement>();
  let cm: EditorHandle | null = null;
  let cmMod = $state<typeof import("../../util/codemirror") | null>(null);
  let note = $state("");
  let question = $state("");

  onMount(async () => {
    cmMod = await import("../../util/codemirror");
  });

  // The editor exists once the file is read (rev > 0); replacing the text bumps rev.
  const ready = $derived(!!cmMod && !!doc && !doc.loading && doc.rev > 0);
  $effect(() => {
    if (!ready || !cmMod || !host) return;
    const m = cmMod;
    const ed = untrack(() =>
      m.createEditor(host!, {
        doc: doc.text,
        path: doc.path,
        onChange: (text) => {
          const d = editors.docs[docId];
          if (d) d.text = text;
        },
        onSave: () => void save(),
      }),
    );
    cm = ed;
    return () => {
      ed.destroy();
      cm = null;
    };
  });
  $effect(() => {
    void doc?.rev;
    untrack(() => doc && cm?.setDoc(doc.text));
  });
  $effect(() => {
    if (focused && !pending) requestAnimationFrame(() => cm?.focus());
  });

  // The proposal's diff against the current editor text.
  $effect(() => {
    const p = doc?.proposal;
    if (!pending || !p || !cmMod || !diffHost) return;
    const view = untrack(() => cmMod!.createDiff(diffHost!, { original: doc.text, proposed: p.text, path: doc.path }));
    return () => view.destroy();
  });

  async function save(force = false) {
    if (!doc || doc.saving) return;
    await editors.save(docId, force);
  }

  async function saveAsRoot() {
    try {
      const r = await sudo.elevate(doc.fs);
      if (!r) return;
      editors.switchFs(docId, r);
      await editors.save(docId);
    } catch (e) {
      toasts.error(e);
    }
  }

  async function reload() {
    if (dirty && !(await ui.ask(t("edit.discardTitle", { name: doc.name }), { body: t("edit.discardBody"), confirmLabel: t("edit.discard"), danger: true }))) return;
    await editors.load(docId);
  }

  function reject() {
    editors.reject(docId, note.trim() || null);
    note = "";
  }

  async function ask(ev: SubmitEvent) {
    ev.preventDefault();
    const q = question.trim();
    const sid = doc?.fs.sessionId;
    if (!q || !sid) return;
    const msg = editorQuestion({ path: doc.path, root: !!doc.fs.root, text: doc.text, dirty }, q, cm?.selection() ?? null);
    question = "";
    if (ui.narrow) ui.aiDrawer = true;
    else settings.aiOpen = true;
    await agent.send(sid, msg, `${q}\n\n— ${doc.name}`);
  }
</script>

{#if doc}
  <div class="editor">
    <div class="bar">
      <Icon name="file" size={14} />
      <span class="path mono" title={doc.path}>
        <span class="dir">{doc.path.slice(0, doc.path.length - doc.name.length)}</span><span class="name">{doc.name}</span>
      </span>
      {#if doc.fs.root}<span class="pill warn">root</span>{/if}
      {#if !doc.stamp && !doc.loading && !doc.error}<span class="pill">{t("edit.newFile")}</span>{/if}
      {#if dirty}<span class="dirty" title={t("edit.unsaved")} aria-label={t("edit.unsaved")}>●</span>{/if}
      <span class="grow"></span>
      <button class="icon-btn" title={t("edit.reload")} aria-label={t("edit.reload")} disabled={doc.loading || !doc.stamp} onclick={reload}><Icon name="refresh" size={14} /></button>
      <button class="btn sm primary" title={t("edit.saveHint", { name: doc.name })} disabled={!dirty || doc.saving || pending} onclick={() => save()}>
        {doc.saving ? t("edit.saving") : t("edit.save")}
      </button>
    </div>

    {#if pending && doc.proposal}
      <div class="banner proposal" role="status">
        <Icon name="sparkle" size={14} />
        <div class="what">
          <strong>{t("edit.proposalBy", { actor: doc.proposal.actor })}</strong>
          {#if doc.proposal.reason}<span>{doc.proposal.reason}</span>{/if}
        </div>
        <input class="input note" bind:value={note} placeholder={t("edit.notePlaceholder")} />
        <button class="btn sm ghost" onclick={reject}>{t("edit.reject")}</button>
        <button class="btn sm primary" disabled={doc.saving} onclick={() => editors.accept(docId)}>{t("edit.accept")}</button>
      </div>
    {/if}
    {#if doc.conflict}
      <div class="banner warn" role="alert">
        <span class="grow">{t("edit.conflict")}</span>
        <button class="btn sm ghost" onclick={() => editors.load(docId)}>{t("edit.reloadDiscard")}</button>
        <button class="btn sm" onclick={() => save(true)}>{t("edit.overwrite")}</button>
      </div>
    {/if}
    {#if doc.error && doc.rev > 0}
      <div class="banner err" role="alert">
        <span class="grow">{doc.error}</span>
        {#if doc.denied && doc.fs.asRoot && !doc.fs.root}<button class="btn sm" onclick={saveAsRoot}><Icon name="shield" size={13} />{t("edit.saveAsRoot")}</button>{/if}
        {#if doc.proposal?.accepted}<button class="btn sm ghost" onclick={() => editors.reject(docId, null)}>{t("edit.giveUp")}</button>{/if}
      </div>
    {/if}

    <div class="body">
      {#if doc.loading}
        <div class="msg">{t("edit.loading")}</div>
      {:else if doc.error && doc.rev === 0}
        <div class="msg err">{doc.error}</div>
      {/if}
      <div class="cm" class:hidden={pending} bind:this={host}></div>
      {#if pending}<div class="cm" bind:this={diffHost}></div>{/if}
    </div>

    {#if doc.fs.sessionId}
      <form class="ask" onsubmit={ask}>
        <Icon name="sparkle" size={13} />
        <input bind:value={question} placeholder={t("edit.askPlaceholder")} aria-label={t("edit.askPlaceholder")} />
        <button class="btn sm" disabled={!question.trim()}>{t("edit.ask")}</button>
      </form>
    {/if}
  </div>
{/if}

<style>
  .editor {
    container-type: inline-size;
    display: flex;
    flex-direction: column;
    width: 100%;
    height: 100%;
    min-width: 0;
    min-height: 0;
    background: var(--bg);
  }
  .bar {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 8px 6px 12px;
    border-bottom: 1px solid var(--border);
    color: var(--muted);
    min-width: 0;
  }
  .path {
    display: flex;
    min-width: 0;
    white-space: nowrap;
    font-size: var(--fs-meta);
  }
  .path .dir {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    color: var(--faint);
  }
  .path .name {
    flex: none;
    color: var(--text);
  }
  .dirty {
    color: var(--accent);
    font-size: 10px;
  }
  .grow {
    flex: 1;
  }
  .banner {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 7px 10px 7px 12px;
    font-size: var(--fs-meta);
    border-bottom: 1px solid var(--border);
  }
  .banner.proposal {
    background: var(--accent-soft);
    color: var(--accent-text);
    flex-wrap: wrap;
  }
  .banner .what {
    display: flex;
    flex-direction: column;
    gap: 1px;
    flex: 1;
    min-width: 160px;
  }
  .banner .what span {
    color: var(--text);
  }
  .banner .note {
    width: 220px;
    max-width: 100%;
    height: 26px;
  }
  .banner.warn {
    background: var(--warn-soft);
    color: var(--warn);
  }
  .banner.err {
    background: var(--danger-soft);
    color: var(--danger);
  }
  .body {
    position: relative;
    flex: 1;
    min-height: 0;
  }
  .cm {
    position: absolute;
    inset: 0;
  }
  .cm.hidden {
    visibility: hidden;
  }
  .msg {
    padding: 16px;
    color: var(--muted);
    font-size: var(--fs-meta);
  }
  .msg.err {
    color: var(--danger);
  }
  .ask {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 8px 6px 12px;
    border-top: 1px solid var(--border);
    color: var(--accent);
  }
  .ask input {
    flex: 1;
    min-width: 0;
    height: 28px;
    border: 0;
    outline: 0;
    background: none;
    color: var(--text);
  }
  @container (max-width: 420px) {
    .banner .note {
      width: 100%;
      order: 3;
    }
  }
</style>
