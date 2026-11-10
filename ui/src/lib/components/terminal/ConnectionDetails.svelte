<script lang="ts">
  import Modal from "../common/Modal.svelte";
  import Icon from "../icons/Icon.svelte";
  import { t } from "../../i18n";
  import { connections } from "../../stores/connections.svelte";
  import { ui } from "../../stores/ui.svelte";
  import { jumpNames, loginUser, sessionDetails } from "./sessionActions.svelte";

  /** Read-only details of the connection behind a terminal; "Edit" opens the editor. */
  const c = $derived(sessionDetails.connectionId ? connections.byId.get(sessionDetails.connectionId) : undefined);

  function edit() {
    if (!c) return;
    const conn = c;
    sessionDetails.close();
    ui.edit(conn);
  }
</script>

{#if c}
  <Modal label={t("sess.detailsTitle", { name: c.name })} onclose={() => sessionDetails.close()} width="460px">
    <div class="dialog">
      <h2>
        <span class="name">{c.name}</span>
        {#if c.env === "prod"}<span class="env-prod-badge">{t("sess.prod")}</span>{/if}
      </h2>
      <dl>
        <dt>{t("sess.fHost")}</dt>
        <dd class="mono">{c.host}</dd>
        <dt>{t("sess.fPort")}</dt>
        <dd class="mono">{c.port}</dd>
        <dt>{t("sess.fUser")}</dt>
        <dd class="mono">{loginUser(c) || "—"}</dd>
        <dt>{t("sess.fEnv")}</dt>
        <dd><span class="env env-{c.env}">{t(`env.${c.env}`)}</span></dd>
        <dt>{t("sess.fKind")}</dt>
        <dd>{c.kind.toUpperCase()}</dd>
        <dt>{t("sess.fJump")}</dt>
        <dd>{jumpNames(c).join(" → ") || t("sess.none")}</dd>
        <dt>{t("sess.fTags")}</dt>
        <dd>
          {#if c.tags.length}
            <span class="tags">{#each c.tags as tag (tag)}<span class="pill">{tag}</span>{/each}</span>
          {:else}{t("sess.none")}{/if}
        </dd>
        <dt>{t("sess.fNotes")}</dt>
        <dd class="notes">{c.notes || t("sess.none")}</dd>
      </dl>
      <div class="actions">
        <button type="button" class="btn ghost" onclick={() => sessionDetails.close()}>{t("common.close")}</button>
        <button type="button" class="btn primary" data-autofocus onclick={edit}><Icon name="edit" size={13} />{t("sess.edit")}</button>
      </div>
    </div>
  </Modal>
{/if}

<style>
  .dialog {
    display: grid;
    gap: var(--sp-4);
    padding: var(--sp-5);
  }
  h2 {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    margin: 0;
    font-size: var(--fs-heading);
    min-width: 0;
  }
  .name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  dl {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: var(--sp-2) var(--sp-4);
    margin: 0;
    font-size: var(--fs-meta);
  }
  dt {
    color: var(--muted);
  }
  dd {
    margin: 0;
    color: var(--text);
    min-width: 0;
    overflow-wrap: anywhere;
  }
  .notes {
    white-space: pre-wrap;
    max-height: 9em;
    overflow: auto;
  }
  .tags {
    display: flex;
    flex-wrap: wrap;
    gap: var(--sp-1);
  }
  .env {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .env::before {
    content: "";
    width: 3px;
    height: 10px;
    border-radius: 2px;
    background: var(--faint);
  }
  .env-prod::before {
    background: var(--env-prod);
  }
  .env-staging::before {
    background: var(--env-staging);
  }
  .env-dev::before {
    background: var(--env-dev);
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: var(--sp-2);
  }
</style>
