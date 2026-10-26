<script lang="ts">
  import Icon from "../icons/Icon.svelte";
  import { t } from "../../i18n";
  import { connections } from "../../stores/connections.svelte";
  import { sessions } from "../../stores/sessions.svelte";
  import { settings } from "../../stores/settings.svelte";

  /** A pane that is still logging in: shows progress and, like OpenSSH, asks for the password
   *  (or a one-time code) the server wants, without echoing it. */
  let { ref, focused, onclose }: { ref: string; focused: boolean; onclose: () => void } = $props();

  const p = $derived(sessions.pending[ref]);
  /** SSH to the RDP port: the server drops the connection, which says nothing on its own. */
  const rdpPort = $derived(connections.byId.get(p?.connectionId ?? "")?.port === 3389);
  let answers = $state<string[]>([]);
  let value = $state("");
  let input = $state<HTMLInputElement>();

  // A new login question starts over.
  $effect(() => {
    void p?.prompt?.request_id;
    answers = [];
    value = "";
  });
  const current = $derived(p?.prompt?.prompts[answers.length] ?? null);
  $effect(() => {
    if (focused && current) requestAnimationFrame(() => input?.focus());
  });

  function key(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      void sessions.answerAuth(ref, null);
    } else if (e.key === "Enter") {
      e.preventDefault();
      const next = [...answers, value];
      value = "";
      if (next.length < (p?.prompt?.prompts.length ?? 0)) answers = next;
      else void sessions.answerAuth(ref, next);
    }
  }
</script>

<div
  class="connecting"
  style:background={settings.terminalTheme.background}
  style:color={settings.terminalTheme.foreground}
  role="presentation"
  onpointerup={() => input?.focus()}
>
  <div class="line dim">{t("connecting.to", { target: p?.target ?? "…" })}</div>
  {#if p?.prompt}
    {#if p.prompt.instructions}
      {#each p.prompt.instructions.split("\n") as line, i (i)}<div class="line">{line}</div>{/each}
    {/if}
    {#each p.prompt.prompts.slice(0, answers.length) as q, i (i)}
      <div class="line">{q.prompt}{q.echo ? answers[i] : ""}</div>
    {/each}
    {#if current}
      <label class="line">
        <span>{current.prompt}</span>
        <input
          bind:this={input}
          bind:value
          class:hidden={!current.echo}
          type={current.echo ? "text" : "password"}
          autocomplete="off"
          autocapitalize="off"
          spellcheck="false"
          aria-label={current.prompt.trim()}
          onkeydown={key}
        />
      </label>
      <div class="hint">{t("connecting.hint")}</div>
    {/if}
  {:else if p?.error}
    <div class="line err">{p.error}</div>
    {#if rdpPort}<div class="line dim">{t("connecting.rdpPort")}</div>{/if}
    <div class="actions">
      <button class="btn sm" onclick={() => sessions.retry(ref)}><Icon name="refresh" size={13} />{t("connecting.retry")}</button>
      <button class="btn sm ghost" onclick={onclose}>{t("connecting.close")}</button>
    </div>
  {:else}
    <div class="line dim working">…</div>
  {/if}
</div>

<style>
  .connecting {
    width: 100%;
    height: 100%;
    padding: 10px 14px;
    font-family: var(--font-mono);
    font-size: 13px;
    line-height: 1.5;
    overflow: auto;
    cursor: text;
  }
  .line {
    display: flex;
    white-space: pre-wrap;
    word-break: break-all;
  }
  .dim {
    opacity: 0.6;
  }
  .err {
    color: var(--danger);
  }
  .working {
    animation: pulse 1.6s var(--ease) infinite;
  }
  input {
    flex: 1;
    min-width: 4ch;
    padding: 0;
    border: 0;
    outline: 0;
    background: transparent;
    color: inherit;
    font: inherit;
  }
  /* No echo, like a terminal password prompt: only the caret shows. */
  input.hidden {
    color: transparent;
    caret-color: currentColor;
  }
  .hint {
    margin-top: 6px;
    opacity: 0.45;
    font-size: 11.5px;
  }
  .actions {
    display: flex;
    gap: 8px;
    margin-top: 10px;
    font-family: var(--font-sans, inherit);
  }
</style>
