<script lang="ts">
  import Modal from "../common/Modal.svelte";
  import { t } from "../../i18n";
  import { api } from "../../api";
  import type { ApprovalMode, SessionInfo } from "../../api/types";
  import { canSetApprovalMode, needsTypedConfirm } from "../../util/approval";
  import { sessions } from "../../stores/sessions.svelte";
  import { toasts } from "../../stores/toasts.svelte";

  let { session }: { session: SessionInfo } = $props();
  let confirming = $state(false);
  let typed = $state("");

  const modes: ApprovalMode[] = ["confirm_all", "confirm_writes", "auto"];
  const name = $derived(session.title);

  async function set(mode: ApprovalMode, confirmed: string | null = null) {
    if (mode === session.approval_mode) return;
    if (needsTypedConfirm(session.env, mode) && confirmed == null) {
      typed = "";
      confirming = true;
      return;
    }
    if (!canSetApprovalMode(session.env, mode, name, confirmed)) return;
    confirming = false;
    try {
      await api.setApprovalMode(session.session_id, mode);
      sessions.infos[session.session_id] = { ...session, approval_mode: mode };
    } catch (e) {
      toasts.error(e);
    }
  }
</script>

<div class="seg" role="group" aria-label={t("ai.mode")}>
  {#each modes as m (m)}
    <button aria-pressed={session.approval_mode === m} class:auto={m === "auto"} onclick={() => set(m)}>{t(`ai.mode.${m}`)}</button>
  {/each}
</div>

{#if confirming}
  <Modal label={t("ai.prodAuto.title")} onclose={() => (confirming = false)} tone="danger">
    <form class="pad" onsubmit={(e) => (e.preventDefault(), set("auto", typed))}>
      <h2>{t("ai.prodAuto.title")}</h2>
      <p>{t("ai.prodAuto.body", { name })}</p>
      <input class="input mono" bind:value={typed} placeholder={name} autocomplete="off" spellcheck="false" aria-label={name} />
      <div class="actions">
        <button type="button" class="btn ghost" onclick={() => (confirming = false)}>{t("common.cancel")}</button>
        <button type="submit" class="btn danger" disabled={!canSetApprovalMode(session.env, "auto", name, typed)}>{t("ai.prodAuto.confirm")}</button>
      </div>
    </form>
  </Modal>
{/if}

<style>
  .seg {
    width: 100%;
  }
  .seg button {
    flex: 1;
  }
  .seg button.auto[aria-pressed="true"] {
    color: var(--warn);
  }
  .pad {
    padding: 20px;
  }
  h2 {
    margin: 0 0 6px;
    font-size: 15px;
    font-weight: 600;
    color: var(--danger);
  }
  p {
    margin: 0 0 14px;
    color: var(--text-2);
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 16px;
  }
</style>
