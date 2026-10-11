<script lang="ts">
  import Modal from "../common/Modal.svelte";
  import { t } from "../../i18n";
  import { s3 } from "../../stores/s3.svelte";
  import { sessions } from "../../stores/sessions.svelte";
  import { toasts } from "../../stores/toasts.svelte";
  import { presetOf, S3_PRESETS, type S3Preset } from "../../util/s3";

  let preset = $state<S3Preset>(S3_PRESETS[0]);
  let saving = $state(false);
  $effect(() => {
    const ed = s3.editing;
    if (ed) preset = presetOf(ed.account);
  });

  function choose(id: string) {
    const ed = s3.editing;
    const p = S3_PRESETS.find((x) => x.id === id);
    if (!ed || !p) return;
    preset = p;
    ed.account.endpoint = "";
    ed.account.region = p.region;
    ed.account.path_style = p.pathStyle;
  }

  const ready = $derived.by(() => {
    const ed = s3.editing;
    if (!ed) return false;
    const a = ed.account;
    const needsEndpoint = preset.id !== "aws";
    return !!a.name.trim() && !!a.access_key_id.trim() && (!!ed.secret.trim() || !!a.secret) && (!needsEndpoint || !!a.endpoint.trim());
  });

  async function save(e: SubmitEvent) {
    e.preventDefault();
    const ed = s3.editing;
    if (!ed || !ready) return;
    if (preset.id === "aws") ed.account.endpoint = "";
    saving = true;
    try {
      const saved = await s3.save(ed.account, ed.secret);
      s3.editing = null;
      if (ed.isNew) sessions.openS3(saved.id);
    } catch (err) {
      toasts.error(err);
    } finally {
      saving = false;
    }
  }
</script>

{#if s3.editing}
  {@const ed = s3.editing}
  <Modal label={t(ed.isNew ? "s3.newTitle" : "s3.editTitle")} onclose={() => (s3.editing = null)} width="500px">
    <form class="dialog" onsubmit={save}>
      <h2>{t(ed.isNew ? "s3.newTitle" : "s3.editTitle")}</h2>
      <label class="field">
        <span class="label">{t("s3.provider")}</span>
        <select class="select" value={preset.id} onchange={(e) => choose((e.currentTarget as HTMLSelectElement).value)}>
          {#each S3_PRESETS as p (p.id)}<option value={p.id}>{p.label}</option>{/each}
        </select>
      </label>
      <label class="field">
        <span class="label">{t("s3.name")}</span>
        <input class="input" bind:value={ed.account.name} placeholder={preset.label} data-autofocus />
      </label>
      {#if preset.id !== "aws"}
        <label class="field">
          <span class="label">{t("s3.endpoint")}</span>
          <input class="input mono" bind:value={ed.account.endpoint} placeholder={preset.endpoint} spellcheck="false" autocomplete="off" />
        </label>
      {/if}
      <div class="two">
        <label class="field">
          <span class="label">{t("s3.region")}</span>
          <input class="input mono" bind:value={ed.account.region} placeholder={preset.region} spellcheck="false" autocomplete="off" />
        </label>
        <label class="field">
          <span class="label">{t("s3.bucket")}</span>
          <input
            class="input mono"
            value={ed.account.bucket ?? ""}
            oninput={(e) => (ed.account.bucket = (e.currentTarget as HTMLInputElement).value)}
            placeholder={t("s3.bucketHint")}
            spellcheck="false"
            autocomplete="off"
          />
        </label>
      </div>
      <label class="field">
        <span class="label">{t("s3.accessKey")}</span>
        <input class="input mono" bind:value={ed.account.access_key_id} spellcheck="false" autocomplete="off" />
      </label>
      <label class="field">
        <span class="label">{t("s3.secretKey")}</span>
        <input
          class="input mono"
          type="password"
          bind:value={ed.secret}
          placeholder={ed.account.secret ? t("s3.secretKeep") : ""}
          spellcheck="false"
          autocomplete="new-password"
        />
        <span class="hint">{t("s3.secretHint")}</span>
      </label>
      {#if preset.id !== "aws"}
        <label class="check">
          <input type="checkbox" bind:checked={ed.account.path_style} />
          <span>{t("s3.pathStyle")}</span>
        </label>
      {/if}
      <div class="actions">
        <button type="button" class="btn ghost" onclick={() => (s3.editing = null)}>{t("common.cancel")}</button>
        <button class="btn primary" disabled={!ready || saving}>{t(ed.isNew ? "s3.saveOpen" : "common.save")}</button>
      </div>
    </form>
  </Modal>
{/if}

<style>
  .dialog {
    display: grid;
    gap: 12px;
    padding: 20px;
  }
  .dialog h2 {
    margin: 0;
    font-size: 15px;
    font-weight: 600;
  }
  .two {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 10px;
  }
  .check {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: var(--fs-meta);
    color: var(--muted);
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }
</style>
