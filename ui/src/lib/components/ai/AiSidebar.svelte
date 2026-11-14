<script lang="ts">
  import BusyButton from "../common/BusyButton.svelte";
  import { tick } from "svelte";
  import { segmented } from "../../actions/segmented";
  import Icon from "../icons/Icon.svelte";
  import Markdown from "../common/Markdown.svelte";
  import BrainPicker from "./BrainPicker.svelte";
  import ApprovalModeControl from "./ApprovalModeControl.svelte";
  import ToolCard from "./ToolCard.svelte";
  import ApprovalCard from "./ApprovalCard.svelte";
  import AgentStatusChip from "./AgentStatusChip.svelte";
  import ActivityView from "./ActivityView.svelte";
  import SummaryView from "./SummaryView.svelte";
  import { fmtNumber, t, type MessageKey } from "../../i18n";
  import { api } from "../../api";
  import { agent, chatBusy } from "../../stores/agent.svelte";
  import { dbApprovals } from "../../stores/dbApprovals.svelte";
  import { sessions } from "../../stores/sessions.svelte";
  import { settings } from "../../stores/settings.svelte";
  import { toasts } from "../../stores/toasts.svelte";
  import { ui } from "../../stores/ui.svelte";
  import { agentStatus } from "../../util/agentActivity";
  import { load, save } from "../../util/storage";
  import { targetOf } from "./target";

  const TEMPLATES: { key: string; label: MessageKey; text: MessageKey }[] = [
    { key: "diagnose", label: "tpl.diagnose", text: "tpl.diagnose.text" },
    { key: "disk", label: "tpl.disk", text: "tpl.disk.text" },
    { key: "explain", label: "tpl.explain", text: "tpl.explain.text" },
    { key: "services", label: "tpl.services", text: "tpl.services.text" },
  ];
  type View = "summary" | "activity" | "chat";
  const VIEWS: View[] = ["summary", "activity", "chat"];

  const session = $derived(sessions.contextSession);
  const sid = $derived(session?.session_id ?? null);
  const chat = $derived(sid ? agent.chats[sid] : undefined);
  const brain = $derived(agent.brainFor(sid));
  const chamber = $derived(brain?.harness?.display === "open_chamber");
  /** The harness runs outside this sidebar (a pane or the OpenChamber window). */
  const native = $derived(brain?.harness?.display === "native" || chamber);
  let opening = $state(false);
  const running = $derived(chatBusy(chat));
  const target = $derived(sid ? targetOf(sid) : null);
  const status = $derived(agentStatus(chat?.items ?? [], running, chat?.cancelled ?? false));
  const pending = $derived(sid ? agent.pendingApprovals(sid) : []);
  /** Approvals waiting in other sessions: one line each, so none is lost behind another tab. */
  const elsewhere = $derived.by(() => {
    const by = new Map<string, number>();
    for (const p of agent.allPending) if (p.sessionId !== sid) by.set(p.sessionId, (by.get(p.sessionId) ?? 0) + 1);
    return [...by].map(([sessionId, n]) => ({ sessionId, n, name: targetOf(sessionId)?.name ?? sessionId }));
  });
  /** Agent SQL waiting in database tabs, one line per tab (the request itself is in the tab). */
  const dbWaiting = $derived.by(() => {
    const ids = new Set(dbApprovals.items.map((i) => i.database_id));
    return [...ids].flatMap((id) => (dbApprovals.tabs[id] ? [{ id, ...dbApprovals.tabs[id] }] : []));
  });

  // svelte-ignore state_referenced_locally
  let view = $state<View>(VIEWS.includes(load("ai-view", "chat")) ? load<View>("ai-view", "chat") : "chat");
  function setView(v: View) {
    view = v;
    save("ai-view", v);
  }
  const tabId = $props.id();

  let input = $state("");
  let scroller = $state<HTMLDivElement>();
  let pane = $state<HTMLDivElement>();
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
    if (stick && scroller && view === "chat") tick().then(() => scroller && (scroller.scrollTop = scroller.scrollHeight));
  });

  async function send(text = input) {
    const msg = text.trim();
    if (!msg || !sid || running) return;
    input = "";
    stick = true;
    await agent.send(sid, msg);
    area?.focus();
  }

  /** User-initiated: a normal chat message (visible in Chat, uses tokens). */
  function askSummary() {
    if (!sid || running || native) return;
    setView("chat");
    void send(t("ai.summary.askPrompt"));
  }

  /** Bring the first pending approval into view and focus it, in the view that shows it. */
  async function review() {
    const first = pending[0];
    if (!first) return;
    if (view === "summary") setView("activity");
    await tick();
    const root = view === "chat" ? scroller : pane;
    const card = root?.querySelector<HTMLElement>(`[data-approval="${CSS.escape(first.req.approval_id)}"]`);
    card?.scrollIntoView({ block: "nearest", behavior: "smooth" });
    card?.focus({ preventScroll: true });
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

  async function openChamber() {
    if (!sid || !brain?.harness || opening) return;
    opening = true;
    try {
      await api.openChamberOpen(sid, brain.harness.id);
    } catch (e) {
      toasts.error(e);
    } finally {
      opening = false;
    }
  }

  function close() {
    if (ui.narrow) ui.aiDrawer = false;
    else settings.aiOpen = false;
  }

  const fmt = (n: number) => (n >= 1000 ? `${(n / 1000).toFixed(1)}k` : String(n));
  const viewLabel = (v: View) => t(`ai.view.${v}`);
</script>

<aside class="ai" aria-label={t("ai.title")}>
  <header>
    <BrainPicker sessionId={sid} />
    <span class="grow"></span>
    {#if running && sid && view !== "chat"}
      <button class="icon-btn" title={t("ai.stopRun")} aria-label={t("ai.stopRun")} onclick={() => sid && agent.cancel(sid)}><Icon name="stop" size={13} /></button>
    {/if}
    <button class="icon-btn" title={t("ai.history.open")} aria-label={t("ai.history.open")} onclick={() => ui.openSettings("approvals")}><Icon name="clock" size={14} /></button>
    {#if sid && chat?.items.length && !running}
      <button class="icon-btn" title={t("ai.clear")} aria-label={t("ai.clear")} onclick={() => sid && agent.clear(sid)}><Icon name="trash" size={14} /></button>
    {/if}
    <button class="icon-btn" title={t("ai.close")} aria-label={t("ai.close")} onclick={close}><Icon name="x" /></button>
  </header>

  {#if session}
    <div class="mode">
      <span class="ctx">
        <span class="env env-{session.env}"></span>
        <span class="ctx-name">{target?.name ?? session.title}</span>
        {#if session.env === "prod"}<span class="env-prod-badge" title={t("env.prod")}>{t("ai.prodBadge")}</span>
        {:else if session.env !== "none"}<span class="ctx-env">{t(`env.${session.env}`)}</span>{/if}
        {#if target?.user || target?.host}<span class="ctx-user mono">{target.user ?? ""}{target.user && target.host ? "@" : ""}{target.host ?? ""}</span>{/if}
        <span class="grow"></span>
        {#if brain}<AgentStatusChip {status} live />{/if}
      </span>
      <ApprovalModeControl {session} />
    </div>
  {/if}

  {#if session && brain}
    <div class="views" role="tablist" aria-label={t("ai.view.label")} use:segmented>
      {#each VIEWS as v (v)}
        <button
          role="tab"
          id="{tabId}-tab-{v}"
          aria-selected={view === v}
          aria-controls="{tabId}-panel-{v}"
          onclick={() => setView(v)}
        >
          {viewLabel(v)}
          {#if v === "activity" && pending.length}
            <span class="badge" aria-label={t("ai.view.pendingBadge", { n: pending.length })}>{pending.length}</span>
          {/if}
        </button>
      {/each}
    </div>
  {/if}

  {#if (session && brain && (pending.length || elsewhere.length)) || dbWaiting.length}
    <div class="banners">
      {#each dbWaiting as d (d.id)}
        <div class="banner other">
          <Icon name="database" size={12} />
          <span>{t("ai.pending.db", { name: d.name })}</span>
          <button class="btn sm" onclick={() => sessions.focusSession(d.leaf)}>{t("ai.pending.goDb")}</button>
        </div>
      {/each}
      {#if session && brain && pending.length}
        <div class="banner" role="alert">
          <Icon name="shield" size={13} />
          <span>{pending.length === 1 ? t("ai.pending.one") : t("ai.pending.many", { n: pending.length })}</span>
          <button class="btn sm primary" onclick={review}>{t("ai.pending.review")}</button>
        </div>
      {/if}
      {#each session && brain ? elsewhere : [] as o (o.sessionId)}
        <div class="banner other">
          <Icon name="shield" size={12} />
          <span>{o.n === 1 ? t("ai.pending.other", { name: o.name }) : `${o.name}: ${t("ai.pending.many", { n: o.n })}`}</span>
          <button class="btn sm" onclick={() => sessions.focusSession(o.sessionId)}>{t("ai.pending.go")}</button>
        </div>
      {/each}
    </div>
  {/if}

  {#if session && brain && sid && view !== "chat"}
    <div class="pane" bind:this={pane} role="tabpanel" id="{tabId}-panel-{view}" aria-labelledby="{tabId}-tab-{view}">
      {#if native}
        <div class="external" role="note">
          <Icon name="info" size={13} />
          <div>
            <strong>{t("ai.external.title", { name: brain.label })}</strong>
            <p>{t("ai.external.body")}</p>
          </div>
        </div>
      {/if}
      {#if view === "summary"}
        <SummaryView
          {chat}
          sessionId={sid}
          {target}
          {status}
          agentLabel={chat?.actor ?? (brain.harness ? brain.label : `${brain.label} · ${brain.detail}`)}
          canAsk={!running && !native}
          onask={askSummary}
          onactivity={() => setView("activity")}
        />
      {:else}
        <ActivityView {chat} sessionId={sid} {target} />
      {/if}
    </div>
  {/if}

  <div
    class="transcript"
    hidden={!!(session && brain) && view !== "chat"}
    bind:this={scroller}
    role={session && brain ? "tabpanel" : undefined}
    id={session && brain ? `${tabId}-panel-chat` : undefined}
    aria-labelledby={session && brain ? `${tabId}-tab-chat` : undefined}
    onscroll={() => scroller && (stick = scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight < 40)}
  >
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
    {:else if chamber}
      <div class="empty">
        <Icon name="terminal" size={18} />
        <strong>{t("ai.chamber.title", { name: brain.label })}</strong>
        <p>{t("ai.chamber.body")}</p>
        <BusyButton class="primary sm" onclick={openChamber} busy={opening} busyLabel={t("ai.chamber.starting")}>
          <Icon name="link" size={13} />{t("ai.chamber.open")}
        </BusyButton>
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
        {:else if item.kind === "limit"}
          <div class="limit" role="status">
            <span>{t("ai.stepLimit", { n: item.steps })}</span>
            {#if item === chat?.items.at(-1) && !running}
              <button class="btn sm primary" onclick={() => send(t("ai.continueMsg"))}>{t("ai.continue")}</button>
            {/if}
          </div>
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

  {#if session && brain && !native && view === "chat"}
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
          {#if chat.usage.cost != null}<span>{fmtNumber(chat.usage.cost, { style: "currency", currency: "USD", maximumFractionDigits: 4 })}</span>{/if}
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
    min-width: 0;
  }
  header :global(.picker) {
    min-width: 0;
    flex: 0 1 auto;
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
  .ctx-name {
    flex: 0 0 auto;
    max-width: 45%;
  }
  .ctx-user {
    flex: 0 1 auto;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--faint);
    font-size: var(--fs-small);
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
  .views {
    display: flex;
    gap: 2px;
    margin: 8px 12px 0;
    padding: 2px;
    border-radius: var(--r);
    background: var(--bg);
    border: 1px solid var(--border);
    flex: none;
  }
  .views button {
    flex: 1;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    height: calc(var(--ctl-h-sm) - 4px);
    border-radius: var(--r-sm);
    color: var(--muted);
    font-size: var(--fs-meta);
    white-space: nowrap;
  }
  .views button:hover,
  .views button[aria-selected="true"] {
    color: var(--text);
  }
  .views button:focus-visible {
    box-shadow: var(--focus-ring);
  }
  .badge {
    display: inline-grid;
    place-items: center;
    min-width: 16px;
    height: 16px;
    padding: 0 4px;
    border-radius: 999px;
    background: var(--warn);
    color: var(--bg);
    font-size: var(--fs-tiny);
    font-weight: 700;
    font-variant-numeric: tabular-nums;
  }
  .banners {
    display: grid;
    gap: 6px;
    margin: 8px 12px 0;
    flex: none;
  }
  .banner {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 6px 6px 10px;
    border-radius: var(--r-lg);
    border: 1px solid color-mix(in srgb, var(--warn) 50%, var(--border));
    background: var(--warn-soft);
    color: var(--text);
    font-size: var(--fs-meta);
  }
  .banner :global(svg) {
    flex: none;
    color: var(--warn);
  }
  .banner span {
    flex: 1;
    min-width: 0;
  }
  .banner.other {
    border-color: var(--border);
    background: var(--hover);
    font-size: var(--fs-small);
    color: var(--text-2);
  }
  .banner.other :global(svg) {
    color: var(--muted);
  }
  .pane {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 12px;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .external {
    display: flex;
    gap: 8px;
    padding: 9px 11px;
    border-radius: var(--r-lg);
    border: 1px solid var(--border);
    background: var(--hover);
    font-size: var(--fs-meta);
    color: var(--muted);
  }
  .external :global(svg) {
    flex: none;
    margin-top: 2px;
    color: var(--accent);
  }
  .external strong {
    color: var(--text);
    font-weight: 550;
  }
  .external p {
    margin: 3px 0 0;
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
    font-size: var(--fs-title);
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
    font-size: var(--fs-meta);
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
  .limit {
    display: flex;
    align-items: center;
    gap: 10px;
    flex-wrap: wrap;
    padding: 8px 10px;
    border-radius: var(--r-lg);
    background: var(--hover);
    color: var(--muted);
    font-size: var(--fs-meta);
  }
</style>
