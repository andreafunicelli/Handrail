<script lang="ts">
  import { onMount } from "svelte";
  import Modal from "../common/Modal.svelte";
  import Icon from "../icons/Icon.svelte";
  import { t } from "../../i18n";
  import type { FileEntry } from "../../api/types";
  import { s3 } from "../../stores/s3.svelte";
  import { sessions } from "../../stores/sessions.svelte";
  import { toasts } from "../../stores/toasts.svelte";
  import { transfers } from "../../stores/transfers.svelte";
  import { ui } from "../../stores/ui.svelte";
  import { alreadyExists, canCopy, s3Backend, sftpBackend, type FileBackend } from "../../util/fileBackends";

  /**
   * Copy `entry` from `fs` into a folder of another file system: an open SSH session's server
   * (or the same one), or an S3 account.
   */
  let { fs, entry, onclose }: { fs: FileBackend; entry: FileEntry; onclose: () => void } = $props();

  interface Target {
    /** `FileBackend.key` of the destination. */
    key: string;
    label: string;
    kind: "sftp" | "s3";
    fs: FileBackend;
  }

  /** Where it can go: the browser already open there (its folder, root or not), else a new one. */
  const targets = $derived.by(() => {
    const open = (key: string, make: () => FileBackend) => transfers.locations[key]?.fs ?? make();
    const out: Target[] = [];
    for (const i of Object.values(sessions.infos)) {
      if (!i.connection_id || i.state === "disconnected") continue;
      const key = `sftp:${i.session_id}`;
      const here = key === fs.key ? ` ${t("files.copySame")}` : "";
      out.push({ key, kind: "sftp", label: `${i.title} — ${i.user ?? ""}@${i.host ?? ""}${here}`, fs: open(key, () => sftpBackend(i.session_id)) });
    }
    for (const a of s3.sorted) {
      const key = `s3:${a.id}`;
      out.push({ key, kind: "s3", label: `${a.name} (S3)`, fs: open(key, () => s3Backend(a.id)) });
    }
    return out.filter((x) => canCopy(fs, entry, x.fs));
  });

  // svelte-ignore state_referenced_locally
  let target = $state((targets.find((x) => x.key !== fs.key) ?? targets[0])?.key ?? "");
  let dir = $state("");
  let loadingDir = $state(false);
  const dest = $derived(targets.find((x) => x.key === target) ?? null);

  async function pickDir() {
    const d = dest;
    if (!d) return;
    const open = transfers.locations[d.key];
    if (open) return void (dir = open.path);
    loadingDir = true;
    try {
      dir = await d.fs.home();
    } catch {
      dir = "";
    } finally {
      loadingDir = false;
    }
  }
  onMount(() => {
    if (!s3.list.length) void s3.load().catch(() => {});
    void pickDir();
  });

  /** S3: "/" is the bucket list, not a folder. */
  const dirOk = $derived(!!dir.trim() && !(dest?.kind === "s3" && !dir.trim().replace(/^\/+/, "")));

  async function go(ev: SubmitEvent) {
    ev.preventDefault();
    // Read everything first: closing the dialog drops its props.
    const d = dest;
    const [src, where, item] = [fs, dir.trim(), { path: entry.path, name: entry.name, kind: entry.kind }];
    if (!d || !dirOk) return;
    onclose();
    const run = (overwrite: boolean) => transfers.copy(src, item.path, d.fs, where, overwrite);
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
      toasts.success(t("files.copied", { path, where: d.label }));
    } catch (e) {
      toasts.error(e);
    }
  }
</script>

<Modal label={t("files.copyTitle", { name: entry.name })} {onclose} width="460px">
  <form class="dialog" onsubmit={go}>
    <h2>{t("files.copyTitle", { name: entry.name })}</h2>
    <p class="hint">{t(entry.kind === "dir" ? "files.copyBodyDir" : "files.copyBody")}</p>
    {#if !targets.length}
      <p class="hint">{t("files.copyNoTarget")}</p>
    {:else}
      <label class="field">
        <span class="label">{t("files.copyTo")}</span>
        <select class="select" bind:value={target} onchange={pickDir}>
          {#each targets as x (x.key)}<option value={x.key}>{x.label}</option>{/each}
        </select>
      </label>
      <label class="field">
        <span class="label">{t("files.copyFolder")}{#if dest?.fs.root}<span class="root"><Icon name="shield" size={11} />root</span>{/if}</span>
        <input class="input mono" bind:value={dir} placeholder={loadingDir ? "…" : dest?.kind === "s3" ? "/bucket/folder" : "/"} spellcheck="false" data-autofocus />
        {#if dest?.kind === "s3" && !dirOk && !loadingDir}<span class="hint">{t("files.copyBucket")}</span>{/if}
      </label>
    {/if}
    <div class="actions">
      <button type="button" class="btn ghost" onclick={onclose}>{t("common.cancel")}</button>
      <button class="btn primary" disabled={!dest || !dirOk}>{t("files.copyGo")}</button>
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
