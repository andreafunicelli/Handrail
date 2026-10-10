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

  let { sessionId, focused = false }: { sessionId: string; focused?: boolean } = $props();

  let box: HTMLDivElement;
  const info = $derived(sessions.infos[sessionId]);
  const agentRunning = $derived(info?.state === "agent_running" || !!agent.chats[sessionId]?.runId);
  const actor = $derived(sessions.lastActor[sessionId] ?? agent.brainFor(sessionId)?.label ?? t("ai.title"));

  onMount(() => {
    const host = terminals.get(sessionId);
    box.appendChild(host.el);
    host.open();
    host.term.attachCustomKeyEventHandler((e) => {
      // Esc while the agent drives this terminal = take over (cancel the run) instead of sending ESC.
      if (e.type === "keydown" && e.key === "Escape" && agent.chats[sessionId]?.runId) {
        void agent.cancel(sessionId);
        return false;
      }
      // Leave app shortcuts to the window handler.
      const mod = e.ctrlKey || e.metaKey;
      const k = e.key.toLowerCase();
      if (mod && !e.altKey && (["k", "j", ","].includes(k) || (e.shiftKey && ["t", "w"].includes(k)) || e.key === "Tab")) return false;
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
      });
    };
    const ro = new ResizeObserver(fit);
    ro.observe(box);
    fit();
    return () => {
      ro.disconnect();
      cancelAnimationFrame(raf);
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
  {#if info?.integration === "limited"}
    <span class="badge pill warn" title={t("term.limitedHint")}>{t("term.limited")}</span>
  {/if}
  {#if info?.state === "disconnected"}
    <div class="ended">
      <span>{t("term.disconnected")}</span>
      <button class="btn sm" onclick={reconnect}><Icon name="refresh" size={13} />{t("term.reconnect")}</button>
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
  .badge {
    position: absolute;
    right: 14px;
    bottom: 8px;
    cursor: help;
  }
  .ended {
    position: absolute;
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
  .ended {
    animation-name: none;
  }
</style>
