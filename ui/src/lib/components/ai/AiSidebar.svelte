<script lang="ts">
  import { tick } from "svelte";
  import Icon from "../icons/Icon.svelte";
  import Markdown from "../common/Markdown.svelte";
  import BrainPicker from "./BrainPicker.svelte";
  import ApprovalModeControl from "./ApprovalModeControl.svelte";
  import ToolCard from "./ToolCard.svelte";
  import ApprovalCard from "./ApprovalCard.svelte";
  import { t, type MessageKey } from "../../i18n";
  import { api } from "../../api";
  import { agent } from "../../stores/agent.svelte";
  import { sessions } from "../../stores/sessions.svelte";
  import { settings } from "../../stores/settings.svelte";
  import { toasts } from "../../stores/toasts.svelte";
  import { ui } from "../../stores/ui.svelte";

  const TEMPLATES: { key: string; label: MessageKey; text: MessageKey }[] = [
    { key: "diagnose", label: "tpl.diagnose", text: "tpl.diagnose.text" },
    { key: "disk", label: "tpl.disk", text: "tpl.disk.text" },
    { key: "explain", label: "tpl.explain", text: "tpl.explain.text" },
    { key: "services", label: "tpl.services", text: "tpl.services.text" },
  ];

  const session = $derived(sessions.contextSession);
  const sid = $derived(session?.session_id ?? null);
  const chat = $derived(sid ? agent.chats[sid] : undefined);
  const brain = $derived(agent.brainFor(sid));
  const native = $derived(brain?.harness?.display === "native");
  const running = $derived(!!chat?.runId);

  let input = $state("");
  let scroller = $state<HTMLDivElement>();
  let area = $state<HTMLTextAreaElement>();
  let tplIndex = $state(0);

  const showTemplates = $derived(input.startsWith("/") && !input.includes("\n"));
  const templates = $derived(
    showTemplates ? TEMPLATES.filter((x) => t(x.label).toLowerCase().includes(input.slice(1).toLowerCase()) || x.key.startsWith(input.slice(1))) : [],
  );

  // Keep the transcript pinned to the bottom while streaming, unless the user scrolled up.
  let stick = true;
  $effect(() => {
    void chat?.items.length;
    const lastItem = chat?.items.at(-1);
    void (lastItem?.kind === "assistant" ? lastItem.text.length : 0);
    if (stick && scroller) tick().then(() => scroller && (scroller.scrollTop = scroller.scrollHeight));
  });

  async function send(text = input) {
    const msg = text.trim();
    if (!msg || !sid || running) return;
    input = "";
    stick = true;
    await agent.send(sid, msg);
    area?.focus();
  }

  function useTemplate(i: number) {
    const tp = templates[i];
    if (!tp) return;
    input = t(tp.text);
    tplIndex = 0;
    area?.focus();
  }

  function key(e: KeyboardEvent) {
    if (templates.length) {
      if (e.key === "ArrowDown" || e.key === "ArrowUp") {
        e.preventDefault();
        tplIndex = (tplIndex + (e.key === "ArrowDown" ? 1 : -1) + templates.length) % templates.length;
        return;
      }
      if (e.key === "Enter" || e.key === "Tab") {
        e.preventDefault();
        useTemplate(tplIndex);
        return;
      }
    }
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      void send();
    }
    if (e.key === "Escape" && running && sid) {
      e.preventDefault();
      void agent.cancel(sid);
    }
  }

  async function openNative() {
    if (!sid || !brain?.harness) return;
    try {
      const info = await api.harnessOpenNative(sid, brain.harness.id, 100, 30);
      sessions.place(info, "row");
    } catch (e) {
      toasts.error(e);
    }
  }

  function close() {
    if (ui.narrow) ui.aiDrawer = false;
    else settings.aiOpen = false;
  }

  const fmt = (n: number) => (n >= 1000 ? `${(n / 1000).toFixed(1)}k` : String(n));
</script>

<aside class="ai" aria-label={t("ai.title")}>
  <header>
    <BrainPicker sessionId={sid} />
    <span class="grow"></span>
    {#if sid && chat?.items.length && !running}
      <button class="icon-btn" title={t("ai.clear")} aria-label={t("ai.clear")} onclick={() => sid && agent.clear(sid)}><Icon name="trash" size={14} /></button>
    {/if}
    <button class="icon-btn" title={t("ai.close")} aria-label={t("ai.close")} onclick={close}><Icon name="x" /></button>
  </header>

  {#if session}
    <div class="mode">
      <span class="ctx">
        <span class="env env-{session.env}"></span>
        <span class="ctx-name">{session.title}</span>
        {#if session.env !== "none"}<span class="ctx-env">{t(`env.${session.env}`)}</span>{/if}
      </span>
      <ApprovalModeControl {session} />
    </div>
  {/if}

  <div class="transcript" bind:this={scroller} onscroll={() => scroller && (stick = scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight < 40)}>
    {#if !session}
      <div class="empty">
        <Icon name="sparkle" size={18} />
        <strong>{t("ai.noSession.title")}</strong>
        <p>{t("ai.noSession.body")}</p>
      </div>
    {:else if !brain}
      <div class="empty">
        <Icon name="sparkle" size={18} />
        <p>{t("ai.noBrains")}</p>
        <button class="btn sm" onclick={() => ui.openSettings("providers")}>{t("ai.openSettings")}</button>
      </div>
    {:else if native}
      <div class="empty">
        <Icon name="terminal" size={18} />
        <strong>{t("ai.native.title", { name: brain.label })}</strong>
        <p>{t("ai.native.body")}</p>
        <button class="btn primary sm" onclick={openNative}><Icon name="splitRight" size={13} />{t("ai.native.open", { name: brain.label })}</button>
      </div>
    {:else if !chat?.items.length}
      <div class="empty">
        <span class="spark">✦</span>
        <strong>{t("ai.empty.title", { name: session.title })}</strong>
        <p>{t("ai.empty.body")}</p>
        <div class="suggest">
          {#each ["ai.suggest.disk", "ai.suggest.load", "ai.suggest.os"] as const as s (s)}
            <button onclick={() => send(t(s))}>{t(s)}</button>
          {/each}
        </div>
      </div>
    {:else if sid}
      {#each chat.items as item (item.id)}
        {#if item.kind === "user"}
          <div class="msg user">{item.text}</div>
        {:else if item.kind === "assistant"}
          <div class="msg assistant">
            {#if item.thinking}<details class="thinking"><summary>{t("ai.thinking")}</summary><p>{item.thinking}</p></details>{/if}
            <Markdown source={item.text} />
          </div>
        {:else if item.kind === "tool"}
          <ToolCard {item} sessionId={sid} />
        {:else if item.kind === "approval"}
          <ApprovalCard {item} sessionId={sid} />
        {:else if item.kind === "error"}
          <div class="err" role="alert">
            <Icon name="alert" size={13} />
            <span>{item.message}</span>
            {#if item.retryable && chat.lastMessage}
              <button class="btn sm" onclick={() => send(chat?.lastMessage ?? "")}>{t("ai.retry")}</button>
            {/if}
          </div>
        {/if}
      {/each}
      {#if running && chat.items.at(-1)?.kind === "user"}
        <div class="typing" aria-label={t("ai.thinking")}><span></span><span></span><span></span></div>
      {/if}
    {/if}
  </div>

  {#if session && brain && !native}
    <div class="composer">
      {#if templates.length}
        <div class="tpls" role="listbox">
          {#each templates as tp, i (tp.key)}
            <button role="option" aria-selected={i === tplIndex} class:on={i === tplIndex} onmousedown={(e) => (e.preventDefault(), useTemplate(i))}>
              <span class="mono">/{tp.key}</span><span class="tl">{t(tp.label)}</span>
            </button>
          {/each}
        </div>
      {/if}
      <div class="box">
        <textarea
          bind:this={area}
          bind:value={input}
          rows="1"
          placeholder={t("ai.input")}
          aria-label={t("ai.input")}
          onkeydown={key}
          oninput={() => (tplIndex = 0)}
        ></textarea>
        {#if running}
          <button class="send stop" title={t("ai.stop")} aria-label={t("ai.stop")} onclick={() => sid && agent.cancel(sid)}><Icon name="stop" size={13} /></button>
        {:else}
          <button class="send" title={t("ai.send")} aria-label={t("ai.send")} disabled={!input.trim()} onclick={() => send()}><Icon name="send" size={14} /></button>
        {/if}
      </div>
      {#if chat && (chat.usage.input || chat.usage.output)}
        <div class="usage">
          <span>{t("ai.tokens", { input: fmt(chat.usage.input), output: fmt(chat.usage.output) })}</span>
          {#if chat.usage.cost != null}<span>${chat.usage.cost.toFixed(4)}</span>{/if}
        </div>
      {/if}
    </div>
  {/if}
</aside>

<style>
  .ai {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-width: 0;
    background: var(--panel);
  }
  header {
    display: flex;
    align-items: center;
    gap: 4px;
    height: var(--header-h);
    padding: 0 6px 0 6px;
    border-bottom: 1px solid var(--border);
    flex: none;
  }
  .grow {
    flex: 1;
  }
  .mode {
    display: grid;
    gap: 8px;
    padding: 10px 12px;
    border-bottom: 1px solid var(--border);
  }
  .ctx {
    display: flex;
    align-items: center;
    gap: 7px;
    font-size: var(--fs-meta);
    color: var(--muted);
    min-width: 0;
  }
  .ctx-name {
    color: var(--text-2);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ctx-env {
    color: var(--faint);
  }
  .env {
    width: 3px;
    height: 11px;
    border-radius: 2px;
    background: var(--border-strong);
    flex: none;
  }
  .env-prod {
    background: var(--env-prod);
  }
  .env-staging {
    background: var(--env-staging);
  }
  .env-dev {
    background: var(--env-dev);
  }
  .transcript {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 14px 12px;
  }
  .empty {
    margin: auto 0;
    display: grid;
    justify-items: start;
    gap: 6px;
    padding: 8px 4px;
    color: var(--muted);
  }
  .empty :global(svg),
  .spark {
    color: var(--accent);
  }
  .spark {
    font-size: 15px;
  }
  .empty strong {
    color: var(--text);
    font-weight: 550;
  }
  .empty p {
    margin: 0 0 6px;
    font-size: var(--fs-meta);
    max-width: 40ch;
  }
  .suggest {
    display: grid;
    gap: 4px;
    width: 100%;
    margin-top: 4px;
  }
  .suggest button {
    text-align: left;
    padding: 7px 10px;
    border-radius: var(--r);
    border: 1px solid var(--border);
    color: var(--text-2);
    font-size: var(--fs-meta);
    transition: border-color var(--t-fast) var(--ease), background var(--t-fast) var(--ease);
  }
  .suggest button:hover {
    border-color: var(--border-strong);
    background: var(--hover);
    color: var(--text);
  }
  .msg.user {
    align-self: flex-end;
    max-width: 88%;
    padding: 7px 11px;
    border-radius: 10px 10px 3px 10px;
    background: var(--active);
    color: var(--text);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .msg.assistant {
    padding: 0 2px;
  }
  .thinking {
    margin-bottom: 6px;
    font-size: var(--fs-meta);
    color: var(--faint);
  }
  .thinking p {
    margin: 4px 0 0;
    white-space: pre-wrap;
  }
  .err {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 10px;
    border-radius: var(--r-lg);
    background: var(--danger-soft);
    color: var(--danger);
    font-size: var(--fs-meta);
  }
  .err span {
    flex: 1;
  }
  .typing {
    display: flex;
    gap: 4px;
    padding: 4px 2px;
  }
  .typing span {
    width: 5px;
    height: 5px;
    border-radius: 50%;
    background: var(--faint);
    animation: pulse 1.2s var(--ease) infinite;
  }
  .typing span:nth-child(2) {
    animation-delay: 0.15s;
  }
  .typing span:nth-child(3) {
    animation-delay: 0.3s;
  }
  .composer {
    position: relative;
    padding: 10px 12px 10px;
    border-top: 1px solid var(--border);
    flex: none;
  }
  .box {
    display: flex;
    align-items: flex-end;
    gap: 6px;
    padding: 6px 6px 6px 10px;
    border-radius: var(--r-lg);
    border: 1px solid var(--border-strong);
    background: var(--bg);
    transition: border-color var(--t-fast) var(--ease), box-shadow var(--t-fast) var(--ease);
  }
  .box:focus-within {
    border-color: var(--accent-line);
    box-shadow: 0 0 0 3px var(--accent-soft);
  }
  textarea {
    flex: 1;
    min-width: 0;
    max-height: 160px;
    field-sizing: content;
    resize: none;
    border: 0;
    outline: 0;
    background: none;
    padding: 3px 0;
    line-height: 1.45;
  }
  textarea::placeholder {
    color: var(--faint);
  }
  .send {
    display: grid;
    place-items: center;
    width: 26px;
    height: 26px;
    border-radius: var(--r);
    background: var(--accent);
    color: var(--on-accent);
    flex: none;
  }
  .send:disabled {
    background: var(--active);
    color: var(--faint);
  }
  .send.stop {
    background: var(--active);
    color: var(--text);
  }
  .tpls {
    position: absolute;
    left: 12px;
    right: 12px;
    bottom: calc(100% - 4px);
    padding: 4px;
    border-radius: var(--r-lg);
    background: var(--raised);
    box-shadow: var(--shadow-float);
  }
  .tpls button {
    display: flex;
    gap: 10px;
    width: 100%;
    padding: 6px 8px;
    border-radius: var(--r-sm);
    text-align: left;
    color: var(--text-2);
    font-size: var(--fs-meta);
  }
  .tpls button.on {
    background: var(--hover);
    color: var(--text);
  }
  .tpls .mono {
    color: var(--accent-text);
    font-size: 11.5px;
    min-width: 72px;
  }
  .usage {
    display: flex;
    justify-content: space-between;
    margin-top: 7px;
    font-size: var(--fs-small);
    color: var(--faint);
    font-variant-numeric: tabular-nums;
  }
</style>
