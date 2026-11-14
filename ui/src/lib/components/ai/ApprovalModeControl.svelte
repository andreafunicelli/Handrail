<script lang="ts">
  import { segmented } from "../../actions/segmented";
  import Modal from "../common/Modal.svelte";
  import { t } from "../../i18n";
  import { api } from "../../api";
  import type { ApprovalMode, SessionInfo } from "../../api/types";
  import { canSetApprovalMode, effectiveMode, needsTypedConfirm } from "../../util/approval";
  import { sessions } from "../../stores/sessions.svelte";
  import { toasts } from "../../stores/toasts.svelte";

  let { session }: { session: SessionInfo } = $props();
  let confirming = $state(false);
  let typed = $state("");

  const modes: ApprovalMode[] = ["confirm_all", "confirm_writes", "auto"];
  const name = $derived(session.title);
  /** The mode picked here. The session reports the mode enforced, which for Auto on production
   *  is Ask writes: the pick is shown while that is what the session still reports. */
  let chosen = $state<{ id: string; mode: ApprovalMode } | null>(null);
  const current = $derived(
    chosen?.id === session.session_id && effectiveMode(session.env, chosen.mode) === session.approval_mode ? chosen.mode : session.approval_mode,
  );

  async function set(mode: ApprovalMode, confirmed: string | null = null) {
    if (mode === current) return;
    if (needsTypedConfirm(session.env, mode) && confirmed == null) {
      typed = "";
      confirming = true;
      return;
    }
    if (!canSetApprovalMode(session.env, mode, name, confirmed)) return;
    confirming = false;
    try {
      await api.setApprovalMode(session.session_id, mode);
      chosen = { id: session.session_id, mode };
      sessions.infos[session.session_id] = { ...session, approval_mode: effectiveMode(session.env, mode) };
    } catch (e) {
      toasts.error(e);
    }
  }
</script>

<div class="seg" role="group" use:segmented aria-label={t("ai.mode")}>
  {#each modes as m (m)}
    <button aria-pressed={current === m} class:auto={m === "auto"} onclick={() => set(m)}>{t(`ai.mode.${m}`)}</button>
  {/each}
</div>
{#if current !== session.approval_mode}<p class="hint downgraded">{t("ai.mode.prodAutoHint")}</p>{/if}

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
  .downgraded {
    margin: 4px 0 0;
    font-size: var(--fs-small);
    color: var(--text-2);
  }
  .pad {
    padding: 20px;
  }
  h2 {
    margin: 0 0 6px;
    font-size: var(--fs-title);
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
