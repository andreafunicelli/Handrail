<script lang="ts">
  import { onMount } from "svelte";
  import { api } from "../../api";
  import { t } from "../../i18n";
  import { terminals } from "../../terminal/host";
  import { sessions } from "../../stores/sessions.svelte";
  import { agent } from "../../stores/agent.svelte";
  import { settings } from "../../stores/settings.svelte";
  import { toasts } from "../../stores/toasts.svelte";
  import Icon from "../icons/Icon.svelte";
  import ContextMenu, { type MenuEntry } from "../common/ContextMenu.svelte";
  import SessionBar from "./SessionBar.svelte";

  let { sessionId, focused = false }: { sessionId: string; focused?: boolean } = $props();

  let box: HTMLDivElement;
  const info = $derived(sessions.infos[sessionId]);
  const agentRunning = $derived(info?.state === "agent_running" || !!agent.chats[sessionId]?.runId);
  const actor = $derived(sessions.lastActor[sessionId] ?? agent.brainFor(sessionId)?.label ?? t("ai.title"));

  const isMac = typeof navigator !== "undefined" && /Mac|iP/.test(navigator.platform);
  let menu = $state<{ x: number; y: number; items: MenuEntry[] } | null>(null);

  /** xterm's selection is not a DOM selection, so the webview never copies it on its own. */
  async function copySelection(): Promise<boolean> {
    const text = terminals.peek(sessionId)?.term.getSelection();
    if (!text) return false;
    try {
      await navigator.clipboard.writeText(text);
      return true;
    } catch {
      toasts.error(t("term.copyFailed"));
      return false;
    }
  }

  async function paste() {
    const term = terminals.peek(sessionId)?.term;
    try {
      const text = await navigator.clipboard.readText();
      if (text && term) term.paste(text);
    } catch {
      toasts.error(t("term.pasteFailed"));
    }
    term?.focus();
  }

  /** The menu focuses its first item; typing must go back to the terminal when it closes. */
  function closeMenu() {
    menu = null;
    terminals.peek(sessionId)?.term.focus();
  }

  /** The terminal's own right-click menu (the webview's is turned off app-wide). */
  function openMenu(e: MouseEvent) {
    e.preventDefault();
    e.stopPropagation();
    const term = terminals.peek(sessionId)?.term;
    if (!term) return;
    menu = {
      x: e.clientX,
      y: e.clientY,
      items: [
        { label: t("edit.copy"), icon: "copy", disabled: !term.hasSelection(), shortcut: isMac ? "⌘C" : "Ctrl+Shift+C", action: () => void copySelection() },
        { label: t("edit.paste"), shortcut: isMac ? "⌘V" : "Ctrl+Shift+V", action: () => void paste() },
        { label: t("edit.selectAll"), action: () => term.selectAll() },
        "sep",
        { label: t("term.clear"), action: () => term.clear() },
      ],
    };
  }

  onMount(() => {
    const host = terminals.get(sessionId);
    box.appendChild(host.el);
    host.open();
    host.el.addEventListener("contextmenu", openMenu, true);
    host.term.attachCustomKeyEventHandler((e) => {
      // Esc while the agent drives this terminal = take over (cancel the run) instead of sending ESC.
      if (e.type === "keydown" && e.key === "Escape" && agent.chats[sessionId]?.runId) {
        void agent.cancel(sessionId);
        return false;
      }
      const mod = e.ctrlKey || e.metaKey;
      const k = e.key.toLowerCase();
      // Copy: Ctrl+Shift+C / Ctrl+Insert (⌘C on macOS). Ctrl+C with a selection copies too, as
      // in Windows Terminal; without one it stays an interrupt.
      const copyKey = isMac
        ? e.metaKey && !e.ctrlKey && !e.altKey && k === "c"
        : !e.altKey && !e.metaKey && e.ctrlKey && ((e.shiftKey && k === "c") || (!e.shiftKey && k === "insert"));
      const ctrlC = !isMac && e.ctrlKey && !e.shiftKey && !e.altKey && !e.metaKey && k === "c" && host.term.hasSelection();
      if (copyKey || ctrlC) {
        e.preventDefault();
        // Ctrl+C clears the selection only once it is on the clipboard, so a failed copy can be retried.
        if (e.type === "keydown") void copySelection().then((ok) => ok && ctrlC && host.term.clearSelection());
        return false;
      }
      // Leave app shortcuts to the window handler.
      if (mod && !e.altKey && (["k", "j", ","].includes(k) || (e.shiftKey && ["t", "w", "enter"].includes(k)) || e.key === "Tab")) return false;
      return true;
    });

    let last = { cols: 0, rows: 0 };
    let raf = 0;
    const fit = () => {
      cancelAnimationFrame(raf);
      raf = requestAnimationFrame(() => {
        if (!box.isConnected || box.clientWidth < 20 || box.clientHeight < 20) return;
        try {
          host.fit.fit();
        } catch {
          return;
        }
        const { cols, rows } = host.term;
        if (cols !== last.cols || rows !== last.rows) {
          last = { cols, rows };
          void api.sessionResize(sessionId, cols, rows).catch(() => {});
        }
        host.term.refresh(0, rows - 1);
      });
    };
    const ro = new ResizeObserver(fit);
    ro.observe(box);
    fit();
    // Re-render when the device pixel ratio changes (moving between monitors, zoom).
    let dprQuery = matchMedia(`(resolution: ${devicePixelRatio}dppx)`);
    const onDpr = () => {
      fit();
      dprQuery.removeEventListener("change", onDpr);
      dprQuery = matchMedia(`(resolution: ${devicePixelRatio}dppx)`);
      dprQuery.addEventListener("change", onDpr);
    };
    dprQuery.addEventListener("change", onDpr);
    return () => {
      dprQuery.removeEventListener("change", onDpr);
      ro.disconnect();
      cancelAnimationFrame(raf);
      host.el.removeEventListener("contextmenu", openMenu, true);
      host.el.remove(); // keep the xterm instance alive in the registry
    };
  });

  $effect(() => {
    // re-fit when font size changes
    void settings.fontSize;
    requestAnimationFrame(() => {
      try {
        terminals.peek(sessionId)?.fit.fit();
      } catch {
        /* not visible */
      }
    });
  });

  $effect(() => {
    if (focused) requestAnimationFrame(() => terminals.peek(sessionId)?.term.focus());
  });

  const rc = $derived(sessions.reconnects[sessionId]);
  // Ticks the countdown to the next attempt.
  let now = $state(Date.now());
  $effect(() => {
    if (!rc?.nextAt) return;
    now = Date.now();
    const timer = setInterval(() => (now = Date.now()), 500);
    return () => clearInterval(timer);
  });
  const endedText = $derived.by(() => {
    const end = info?.ended;
    if (end?.exit_status != null) return t("term.exited", { code: end.exit_status });
    if (rc && !rc.running && rc.attempt > 0) return t("term.reconnectFailed", { reason: rc.error ?? "" });
    if (end?.error) return t("term.lost", { reason: end.error });
    return t("term.disconnected");
  });

  async function reconnect() {
    try {
      await sessions.reconnect(sessionId);
    } catch (e) {
      toasts.error(e);
    }
  }
</script>

<div class="term" style:background={settings.terminalTheme.background}>
  {#if agentRunning}
    <div class="banner" role="status">
      <span class="spark">✦</span>
      <span>{t("term.agentBanner", { actor })}</span>
      <span class="grow"></span>
      <button class="take" onclick={() => agent.cancel(sessionId)}><span class="kbd">Esc</span> {t("term.takeOver")}</button>
    </div>
  {/if}
  <div class="xterm-box" bind:this={box}></div>
  {#if menu}<ContextMenu {...menu} onclose={closeMenu} />{/if}
  {#if info?.connection_id && settings.sessionBar}<SessionBar {sessionId} />{/if}
  {#if info?.integration === "limited"}
    <span class="badge pill warn" title={t("term.limitedHint")}>{t("term.limited")}</span>
  {/if}
  {#if info?.state === "disconnected"}
    <div class="ended" role="status">
      {#if rc?.running}
        <span class="spin" aria-hidden="true"><Icon name="refresh" size={13} /></span>
        <span class="msg" title={rc.error ?? ""}>
          {rc.nextAt ? t("term.reconnectingIn", { s: Math.max(1, Math.ceil((rc.nextAt - now) / 1000)) }) : t("term.reconnecting")}
          {#if rc.attempt}<span class="faint"> · {t("term.attempt", { n: rc.attempt })}</span>{/if}
        </span>
        <button class="btn sm" onclick={() => sessions.reconnectNow(sessionId)}>{t("term.reconnectNow")}</button>
        <button class="btn sm ghost" onclick={() => sessions.stopReconnect(sessionId)}>{t("term.stopReconnect")}</button>
      {:else}
        <span class="msg" title={endedText}>{endedText}</span>
        <button class="btn sm" onclick={reconnect}><Icon name="refresh" size={13} />{t("term.reconnect")}</button>
      {/if}
    </div>
  {/if}
</div>

<style>
  .term {
    position: relative;
    display: flex;
    flex-direction: column;
    width: 100%;
    height: 100%;
    min-width: 0;
    min-height: 0;
  }
  .xterm-box {
    flex: 1;
    min-height: 0;
    padding: 8px 4px 4px 18px;
    overflow: hidden;
  }
  .banner {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 26px;
    padding: 0 6px 0 12px;
    font-size: var(--fs-meta);
    color: var(--accent-text);
    background: var(--accent-soft);
    border-bottom: 1px solid var(--accent-line);
    animation: drop var(--t) var(--ease);
  }
  .spark {
    font-size: 10px;
    animation: pulse 1.4s var(--ease) infinite;
  }
  .grow {
    flex: 1;
  }
  .take {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 20px;
    padding: 0 6px;
    border-radius: var(--r-sm);
    color: var(--text-2);
    font-size: var(--fs-small);
  }
  .take:hover {
    background: var(--accent-soft);
    color: var(--text);
  }
  .take .kbd {
    height: 16px;
    font-size: 9.5px;
  }
  /* Overlays above xterm's own layers (link layer z-index 2, helpers 5, decorations 6-8,
     message 10, accessibility 11), or those swallow the clicks (issue #33). */
  .badge {
    position: absolute;
    right: 14px;
    bottom: 8px;
    z-index: 20;
    cursor: help;
  }
  .ended {
    position: absolute;
    z-index: 20;
    left: 50%;
    bottom: 18px;
    transform: translateX(-50%);
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 6px 6px 6px 14px;
    border-radius: var(--r-lg);
    background: var(--raised);
    box-shadow: var(--shadow-float);
    color: var(--text-2);
    font-size: var(--fs-meta);
    animation: drop var(--t) var(--ease);
  }
  @keyframes drop {
    from {
      opacity: 0;
      transform: translateY(-3px);
    }
  }
  .ended,
  .spin {
    animation-name: none;
  }
  .ended .msg {
    max-width: 46ch;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ended .faint {
    color: var(--faint);
  }
  .spin {
    display: inline-flex;
    color: var(--accent);
    animation: spin 1.2s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
</style>
