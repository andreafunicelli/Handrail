<script lang="ts">
  import Icon from "../icons/Icon.svelte";
  import Modal from "../common/Modal.svelte";
  import { t } from "../../i18n";
  import { editorOffers } from "../../stores/editorOffer.svelte";

  /** `nano file` typed in a terminal: open the file in Handrail's editor, or run the command. */
  let remember = $state(false);
</script>

{#if editorOffers.offer}
  {@const o = editorOffers.offer}
  <Modal label={t("editorOffer.title")} onclose={() => editorOffers.dismiss()} width="440px">
    <div class="dialog">
      <h2>{t("editorOffer.title")}</h2>
      <p class="file mono" title={o.path}>
        <Icon name="file" size={14} /><span>{o.path}</span>{#if o.root}<span class="pill warn">root</span>{/if}
      </p>
      <p class="hint">{t("editorOffer.body", { editor: o.editor })}</p>
      <label class="check">
        <input type="checkbox" bind:checked={remember} />
        <span>{t("editorOffer.remember")}</span>
      </label>
      <div class="actions">
        <button type="button" class="btn ghost" onclick={() => editorOffers.runInTerminal(o, remember)}>{t("editorOffer.run", { editor: o.editor })}</button>
        <button type="button" class="btn primary" data-autofocus onclick={() => editorOffers.openInEditor(o, remember)}>
          <Icon name="file" size={13} />{t("editorOffer.open")}
        </button>
      </div>
    </div>
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
  .file {
    display: flex;
    align-items: center;
    gap: 8px;
    margin: 0;
    min-width: 0;
  }
  .file span:first-of-type {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .hint {
    margin: 0;
  }
  .check {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }
</style>
