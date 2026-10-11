<script lang="ts">
  import { onMount } from "svelte";
  import Modal from "../common/Modal.svelte";
  import Icon from "../icons/Icon.svelte";
  import { t } from "../../i18n";
  import type { FileEntry } from "../../api/types";
  import { sessions } from "../../stores/sessions.svelte";
  import { toasts } from "../../stores/toasts.svelte";
  import { transfers } from "../../stores/transfers.svelte";
  import { ui } from "../../stores/ui.svelte";
  import { alreadyExists, sftpBackend, type FileBackend } from "../../util/fileBackends";

  /** Copy `entry` from `fs` to a folder on another open SSH session (or the same one). */
  let { fs, entry, onclose }: { fs: FileBackend; entry: FileEntry; onclose: () => void } = $props();

  const targets = $derived(Object.values(sessions.infos).filter((i) => i.connection_id && i.state !== "disconnected"));
  // svelte-ignore state_referenced_locally
  let target = $state(targets.find((i) => i.session_id !== fs.sessionId)?.session_id ?? fs.sessionId ?? "");
  let dir = $state("");
  let loadingDir = $state(false);

  /** The destination's file system: its open browser's (root or not), else the login user's. */
  const dest = $derived(transfers.locations[`sftp:${target}`]?.fs ?? sftpBackend(target));

  async function pickDir(sessionId: string) {
    const open = transfers.locations[`sftp:${sessionId}`];
    if (open) return void (dir = open.path);
    loadingDir = true;
    try {
      dir = await sftpBackend(sessionId).home();
    } catch {
      dir = "";
    } finally {
      loadingDir = false;
    }
  }
  onMount(() => void (target && pickDir(target)));

  const label = (id: string) => {
    const i = sessions.infos[id];
    return i ? `${i.title} — ${i.user ?? ""}@${i.host ?? ""}` : id;
  };

  async function go(ev: SubmitEvent) {
    ev.preventDefault();
    // Read everything first: closing the dialog drops its props.
    const [to, where, src, item, whereLabel] = [dest, dir.trim(), fs, { path: entry.path, name: entry.name, kind: entry.kind }, label(target)];
    if (!where || !target) return;
    onclose();
    const run = (overwrite: boolean) => transfers.copy(src, item.path, to, where, overwrite);
    try {
      let path: string;
      try {
        path = await run(false);
      } catch (e) {
        if (!alreadyExists(e)) throw e;
        const ok = await ui.ask(t("files.copyExists", { name: item.name }), {
          body: t(item.kind === "dir" ? "files.copyMerge" : "files.copyReplace"),
          confirmLabel: t("files.copyOverwrite"),
          danger: true,
        });
        if (!ok) return;
        path = await run(true);
      }
      toasts.success(t("files.copied", { path, where: whereLabel }));
    } catch (e) {
      toasts.error(e);
    }
  }
</script>

<Modal label={t("files.copyTitle", { name: entry.name })} {onclose} width="460px">
  <form class="dialog" onsubmit={go}>
    <h2>{t("files.copyTitle", { name: entry.name })}</h2>
    <p class="hint">{t("files.copyBody")}</p>
    {#if !targets.length}
      <p class="hint">{t("files.copyNoTarget")}</p>
    {:else}
      <label class="field">
        <span class="label">{t("files.copyTo")}</span>
        <select class="select" bind:value={target} onchange={() => pickDir(target)}>
          {#each targets as i (i.session_id)}<option value={i.session_id}>{label(i.session_id)}{i.session_id === fs.sessionId ? ` ${t("files.copySame")}` : ""}</option>{/each}
        </select>
      </label>
      <label class="field">
        <span class="label">{t("files.copyFolder")}{#if dest.root}<span class="root"><Icon name="shield" size={11} />root</span>{/if}</span>
        <input class="input mono" bind:value={dir} placeholder={loadingDir ? "…" : "/"} spellcheck="false" data-autofocus />
      </label>
    {/if}
    <div class="actions">
      <button type="button" class="btn ghost" onclick={onclose}>{t("common.cancel")}</button>
      <button class="btn primary" disabled={!targets.length || !dir.trim()}>{t("files.copyGo")}</button>
    </div>
  </form>
</Modal>

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
    overflow-wrap: anywhere;
  }
  .hint {
    margin: 0;
  }
  .root {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    margin-left: 8px;
    color: var(--warn);
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }
</style>
