<script lang="ts">
  import BusyButton from "../common/BusyButton.svelte";
  import { onMount, untrack } from "svelte";
  import Icon from "../icons/Icon.svelte";
  import ContextMenu, { type MenuEntry } from "../common/ContextMenu.svelte";
  import { t } from "../../i18n";
  import { agent } from "../../stores/agent.svelte";
  import { editors } from "../../stores/editors.svelte";
  import { settings } from "../../stores/settings.svelte";
  import { sudo } from "../../stores/sudo.svelte";
  import { toasts } from "../../stores/toasts.svelte";
  import { ui } from "../../stores/ui.svelte";
  import { editorQuestion } from "../../util/editorAsk";
  import type { EditorCommand, EditorHandle, EditorStatus, Eol, Indent, ViewPrefs } from "../../util/codemirror";

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
  let askInput = $state<HTMLInputElement>();
  let status = $state<EditorStatus | null>(null);
  /** Per file: syntax, indentation and line endings, detected on open and changeable. */
  let language = $state("text");
  /** Detected rather than picked: a plain-text file that gains a `#!` line is detected again. */
  let languageAuto = true;
  let indent = $state<Indent>({ tabs: false, size: 4 });
  let eol = $state<Eol>("LF");
  let menu = $state<{ x: number; y: number; items: MenuEntry[] } | null>(null);

  const mod = typeof navigator !== "undefined" && /Mac|iP/.test(navigator.platform) ? "⌘" : "Ctrl+";
  const prefs = $derived<ViewPrefs>({
    wrap: settings.editorWrap,
    whitespace: settings.editorWhitespace,
    minimap: settings.editorMinimap,
    guides: settings.editorGuides,
    fontSize: settings.editorFontSize,
  });

  onMount(async () => {
    cmMod = await import("../../util/codemirror");
  });

  // The editor exists once the file is read (rev > 0); replacing the text bumps rev.
  const ready = $derived(!!cmMod && !!doc && !doc.loading && doc.rev > 0);
  $effect(() => {
    if (!ready || !cmMod || !host) return;
    const m = cmMod;
    const ed = untrack(() => {
      language = m.detectLanguage(doc.path, doc.text);
      indent = m.detectIndent(doc.text, { tabs: false, size: language === "yaml" ? 2 : 4 });
      eol = m.detectEol(doc.text);
      return m.createEditor(host!, {
        doc: doc.text,
        path: doc.path,
        prefs,
        language,
        indent,
        onChange: (text) => {
          const d = editors.docs[docId];
          if (d) d.text = text;
          if (languageAuto && language === "text" && text.startsWith("#!") && text.includes("\n")) {
            const id = m.detectLanguage(doc.path, text);
            // Not from inside CodeMirror's update.
            if (id !== "text") queueMicrotask(() => setLanguage(id, true));
          }
        },
        onSave: () => void save(),
        onStatus: (st) => (status = st),
        onView: (a) => {
          if (a === "wrap") settings.editorWrap = !settings.editorWrap;
          else settings.zoomEditor(a === "zoomIn" ? 1 : a === "zoomOut" ? -1 : 0);
        },
        onContextMenu: editMenu,
      });
    });
    cm = ed;
    return () => {
      ed.destroy();
      cm = null;
    };
  });
  $effect(() => {
    void doc?.rev;
    untrack(() => {
      if (!doc || !cm || !cmMod) return;
      cm.setDoc(doc.text);
      eol = cmMod.detectEol(doc.text);
    });
  });
  $effect(() => {
    const p = prefs;
    untrack(() => cm?.setPrefs(p));
  });
  $effect(() => {
    if (focused && !pending) requestAnimationFrame(() => cm?.focus());
  });

  // The proposal's diff against the current editor text.
  $effect(() => {
    const p = doc?.proposal;
    if (!pending || !p || !cmMod || !diffHost) return;
    const view = untrack(() => cmMod!.createDiff(diffHost!, { original: doc.text, proposed: p.text, path: doc.path, fontSize: settings.editorFontSize }));
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

  function command(cmd: EditorCommand) {
    cm?.run(cmd).catch(() => toasts.error(t(cmd === "paste" ? "edit.pasteKeys" : "edit.clipboardFailed", { mod })));
  }

  function setLanguage(id: string, auto = false) {
    languageAuto = auto;
    language = id;
    cm?.setLanguage(id);
  }
  function setIndent(i: Indent) {
    indent = i;
    cm?.setIndent(i);
  }
  function setEol(e: Eol) {
    if (e === eol) return;
    eol = e;
    cm?.setEol(e);
  }

  const at = (ev: MouseEvent) => {
    const r = (ev.currentTarget as HTMLElement | null)?.getBoundingClientRect();
    return ev.clientX || !r ? { x: ev.clientX, y: ev.clientY } : { x: r.left, y: r.bottom + 4 };
  };

  /** Right-click in the text. */
  function editMenu(ev: MouseEvent) {
    const sel = !!cm?.selection();
    menu = {
      x: ev.clientX,
      y: ev.clientY,
      items: [
        { label: t("edit.cut"), shortcut: `${mod}X`, disabled: !sel || pending, action: () => command("cut") },
        { label: t("edit.copy"), icon: "copy", shortcut: `${mod}C`, disabled: !sel, action: () => command("copy") },
        { label: t("edit.paste"), shortcut: `${mod}V`, disabled: pending, action: () => command("paste") },
        "sep",
        { label: t("edit.find"), icon: "search", shortcut: `${mod}F`, action: () => command("find") },
        { label: t("edit.replace"), shortcut: `${mod}H`, action: () => command("replace") },
        { label: t("edit.gotoLine"), icon: "jump", shortcut: `${mod}G`, action: () => command("gotoLine") },
        "sep",
        { label: t("edit.toggleComment"), shortcut: `${mod}/`, action: () => command("comment") },
        { label: t("edit.selectMatches"), shortcut: `${mod}Shift+L`.replace("⌘Shift+", "⇧⌘"), disabled: !sel, action: () => command("selectMatches") },
        { label: t("edit.selectAll"), shortcut: `${mod}A`, action: () => command("selectAll") },
        ...(doc?.fs.sessionId
          ? (["sep", { label: t(sel ? "edit.askSelection" : "edit.askFile"), icon: "sparkle", action: () => askInput?.focus() }] as MenuEntry[])
          : []),
      ],
    };
  }

  function viewMenu(ev: MouseEvent) {
    const p = at(ev);
    menu = {
      ...p,
      items: [
        { label: t("edit.wordWrap"), checked: settings.editorWrap, shortcut: "Alt+Z", action: () => (settings.editorWrap = !settings.editorWrap) },
        { label: t("edit.minimap"), checked: settings.editorMinimap, action: () => (settings.editorMinimap = !settings.editorMinimap) },
        { label: t("edit.guides"), checked: settings.editorGuides, action: () => (settings.editorGuides = !settings.editorGuides) },
        { label: t("edit.whitespace"), checked: settings.editorWhitespace, action: () => (settings.editorWhitespace = !settings.editorWhitespace) },
        "sep",
        { label: t("edit.zoomIn"), shortcut: `${mod}=`, action: () => settings.zoomEditor(1) },
        { label: t("edit.zoomOut"), shortcut: `${mod}-`, action: () => settings.zoomEditor(-1) },
        { label: t("edit.zoomReset", { size: settings.editorFontSize }), shortcut: `${mod}0`, action: () => settings.zoomEditor(0) },
      ],
    };
  }

  function indentMenu(ev: MouseEvent) {
    menu = {
      ...at(ev),
      items: [
        { label: t("edit.indentSpaces"), checked: !indent.tabs, action: () => setIndent({ ...indent, tabs: false }) },
        { label: t("edit.indentTabs"), checked: indent.tabs, action: () => setIndent({ ...indent, tabs: true }) },
        "sep",
        ...[2, 4, 8].map((size) => ({ label: t("edit.tabSize", { size }), checked: indent.size === size, action: () => setIndent({ ...indent, size }) })),
        "sep",
        { label: t("edit.indentDetect"), action: () => cmMod && setIndent(cmMod.detectIndent(doc.text, indent)) },
      ],
    };
  }

  function eolMenu(ev: MouseEvent) {
    menu = {
      ...at(ev),
      items: [
        { label: t("edit.eolLF"), checked: eol === "LF", action: () => setEol("LF") },
        { label: t("edit.eolCRLF"), checked: eol === "CRLF", action: () => setEol("CRLF") },
      ],
    };
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
      <button class="icon-btn" title={`${t("edit.find")} (${mod}F)`} aria-label={t("edit.find")} disabled={!ready || pending} onclick={() => command("find")}><Icon name="search" size={14} /></button>
      <button class="icon-btn" title={t("edit.view")} aria-label={t("edit.view")} aria-haspopup="menu" onclick={viewMenu}><Icon name="layout" size={14} /></button>
      <button class="icon-btn" title={t("edit.reload")} aria-label={t("edit.reload")} disabled={doc.loading || !doc.stamp} onclick={reload}><Icon name="refresh" size={14} /></button>
      <BusyButton class="sm primary" title={t("edit.saveHint", { name: doc.name })} busy={doc.saving} busyLabel={t("edit.saving")} disabled={!dirty || pending} onclick={() => save()}>
        {t("edit.save")}
      </BusyButton>
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

    {#if ready && status && !pending}
      <div class="status mono" role="status">
        <button title={`${t("edit.gotoLine")} (${mod}G)`} onclick={() => command("gotoLine")}>
          {t("edit.position", { line: status.line, col: status.col })}{#if status.selected}&nbsp;{t("edit.selected", { n: status.selected })}{/if}{#if status.cursors > 1}&nbsp;· {t("edit.cursors", { n: status.cursors })}{/if}
        </button>
        <span class="grow"></span>
        <button title={t("edit.indentation")} onclick={indentMenu}>{t(indent.tabs ? "edit.tabs" : "edit.spaces", { size: indent.size })}</button>
        <span class="cell" title={t("edit.encoding")}>UTF-8</span>
        <button title={t("edit.lineEndings")} onclick={eolMenu}>{eol}</button>
        <select title={t("edit.language")} aria-label={t("edit.language")} value={language} onchange={(e) => setLanguage(e.currentTarget.value)}>
          {#each cmMod?.LANGUAGES ?? [] as l (l.id)}<option value={l.id}>{l.name}</option>{/each}
        </select>
      </div>
    {/if}

    {#if doc.fs.sessionId}
      <form class="ask" onsubmit={ask}>
        <Icon name="sparkle" size={13} />
        <input bind:this={askInput} bind:value={question} placeholder={t("edit.askPlaceholder")} aria-label={t("edit.askPlaceholder")} />
        <button class="btn sm" disabled={!question.trim()}>{t("edit.ask")}</button>
      </form>
    {/if}
  </div>
{/if}

{#if menu}<ContextMenu x={menu.x} y={menu.y} items={menu.items} onclose={() => (menu = null)} />{/if}

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
  .status {
    display: flex;
    align-items: center;
    gap: 2px;
    height: 24px;
    padding: 0 6px;
    border-top: 1px solid var(--border);
    background: var(--panel);
    color: var(--muted);
    font-size: 11px;
    white-space: nowrap;
    overflow: hidden;
  }
  .status button,
  .status select,
  .status .cell {
    height: 20px;
    padding: 0 6px;
    border: 0;
    border-radius: 4px;
    background: none;
    color: inherit;
    font: inherit;
  }
  .status .cell {
    display: inline-flex;
    align-items: center;
  }
  .status button:hover,
  .status select:hover {
    background: var(--hover);
    color: var(--text);
  }
  .status select {
    cursor: pointer;
    max-width: 130px;
  }
  .status option {
    background: var(--raised);
    color: var(--text);
  }
  @container (max-width: 480px) {
    .status .cell {
      display: none;
    }
  }
  @container (max-width: 420px) {
    .banner .note {
      width: 100%;
      order: 3;
    }
  }
</style>
