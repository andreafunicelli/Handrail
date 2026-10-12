<script lang="ts">
  import { onMount } from "svelte";
  import Icon from "../icons/Icon.svelte";
  import { t } from "../../i18n";
  import { api } from "../../api";
  import type { ApprovalMode, Connection, Environment, SecretKind } from "../../api/types";
  import { connections } from "../../stores/connections.svelte";
  import { credentials } from "../../stores/credentials.svelte";
  import { sessions } from "../../stores/sessions.svelte";
  import { toasts } from "../../stores/toasts.svelte";
  import { ui } from "../../stores/ui.svelte";
  import { groupPath } from "../../util/tree";
  import { uuid } from "../../util/id";
  import { newTunnel, tunnelProblem } from "../../util/tunnels";

  let { connection, isNew }: { connection: Connection; isNew: boolean } = $props();

  // Local editable copy (the prop is already a detached clone made by ui.edit).
  // svelte-ignore state_referenced_locally
  let c = $state({ ...connection, tunnels: connection.tunnels ?? [] });
  let authType = $state<"password" | "key" | "agent" | "prompt">(c.auth.type);
  let password = $state("");
  let keyText = $state("");
  let passphrase = $state("");
  let tagsText = $state(c.tags.join(", "));
  let busy = $state(false);
  let sheet: HTMLElement;
  let fileInput = $state<HTMLInputElement>();

  /** The saved password or key this connection uses ("" for a new one typed below). */
  let passwordId = $state(c.auth.type === "password" ? c.auth.secret : "");
  let keyId = $state(c.auth.type === "key" ? c.auth.key : "");
  const keptPassphrase = c.auth.type === "key" ? c.auth.passphrase : null;
  /** Other connections using a saved secret. */
  const others = (id: string) => credentials.usedBy(id, c.id);
  const envs: Environment[] = ["none", "dev", "staging", "prod"];
  const jumpCandidates = $derived(connections.list.filter((x) => x.id !== c.id && x.kind === "ssh"));
  const tunnelsOk = $derived(c.tunnels.every((x) => !tunnelProblem(x)));
  const rdp = $derived(c.kind === "rdp");
  /** RDP logs in with a saved password only (NLA needs it before the session starts). */
  const rdpPasswordOk = $derived(!rdp || !!password || !!passwordId);
  const valid = $derived(c.name.trim() !== "" && c.host.trim() !== "" && c.port > 0 && c.port < 65536 && tunnelsOk && rdpPasswordOk);

  function setKind(kind: "ssh" | "rdp") {
    if (c.kind === kind) return;
    if (kind === "rdp" && c.port === 22) c.port = 3389;
    if (kind === "ssh" && c.port === 3389) c.port = 22;
    if (kind === "rdp") authType = "password";
    c.kind = kind;
  }

  onMount(() => {
    sheet.querySelector<HTMLInputElement>("input")?.focus();
    void credentials.load().catch(() => {});
  });

  function close() {
    ui.editing = null;
  }

  async function secret(kind: SecretKind, value: string, label: string) {
    return api.saveSecret({ id: uuid(), kind, label, value });
  }

  async function save(connectAfter = false) {
    if (!valid || busy) return;
    busy = true;
    try {
      const out: Connection = $state.snapshot(c) as Connection;
      out.name = out.name.trim();
      out.host = out.host.trim();
      out.user = out.user.trim();
      out.tags = tagsText.split(",").map((s) => s.trim()).filter(Boolean);
      out.tunnels = out.tunnels.map((x) => ({ ...x, bind_host: x.bind_host.trim(), dest_host: x.kind === "dynamic" ? "" : x.dest_host.trim(), dest_port: x.kind === "dynamic" ? 0 : x.dest_port }));
      if (out.env === "prod" && out.approval_mode === "auto") out.approval_mode = null;
      if (out.kind === "rdp") {
        // Tunnels and agent approvals belong to SSH sessions.
        out.tunnels = [];
        out.approval_mode = null;
      } else {
        out.rdp_admin = false;
      }
      if (authType === "password") {
        // A typed password is new (for this connection only); else the saved one picked.
        if (password) out.auth = { type: "password", secret: await secret("password", password, `${out.name} password`) };
        else if (passwordId) out.auth = { type: "password", secret: passwordId };
        else out.auth = { type: "prompt" };
      } else if (authType === "key") {
        if (keyText.trim()) {
          const key = await secret("private_key", keyText, `${out.name} key`);
          const pp = passphrase ? await secret("passphrase", passphrase, `${out.name} passphrase`) : null;
          out.auth = { type: "key", key, passphrase: pp };
        } else if (keyId) {
          // A saved key keeps the passphrase stored with it, unless a new one is typed.
          const kept = c.auth.type === "key" && c.auth.key === keyId ? keptPassphrase : credentials.passphraseFor(keyId);
          const pp = passphrase ? await secret("passphrase", passphrase, `${credentials.label(keyId) || out.name} passphrase`) : kept;
          out.auth = { type: "key", key: keyId, passphrase: pp };
        } else {
          toasts.error(t("editor.keyPaste"));
          busy = false;
          return;
        }
      } else {
        out.auth = { type: authType };
      }
      password = keyText = passphrase = "";
      const saved = await connections.save(out);
      toasts.success(t("editor.saved", { name: saved.name }));
      close();
      if (connectAfter) await sessions.open(saved.id);
    } catch (e) {
      toasts.error(e);
    } finally {
      busy = false;
    }
  }

  async function pickKey(e: Event) {
    const f = (e.currentTarget as HTMLInputElement).files?.[0];
    if (f) keyText = await f.text();
  }

  function toggleJump(id: string) {
    c.jump = c.jump.includes(id) ? c.jump.filter((j) => j !== id) : [...c.jump, id];
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="scrim" onmousedown={close}></div>
<div
  class="sheet"
  tabindex="-1"
  role="dialog"
  aria-modal="true"
  aria-label={isNew ? t("editor.new") : t("editor.edit")}
  bind:this={sheet}
  onkeydown={(e) => {
    if (e.key === "Escape") {
      e.stopPropagation();
      close();
    }
    if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) save();
  }}
>
  <header>
    <h2>{isNew ? t("editor.new") : t("editor.edit")}</h2>
    <button class="icon-btn" aria-label={t("common.close")} onclick={close}><Icon name="x" /></button>
  </header>

  <form class="body" onsubmit={(e) => (e.preventDefault(), save())}>
    {#if isNew}
      <div class="seg kind" role="group" aria-label={t("editor.kind")}>
        <button type="button" aria-pressed={!rdp} onclick={() => setKind("ssh")}><Icon name="terminal" size={13} />SSH</button>
        <button type="button" aria-pressed={rdp} onclick={() => setKind("rdp")}><Icon name="monitor" size={13} />{t("editor.kind.rdp")}</button>
      </div>
    {:else if rdp}
      <p class="hint kindnote"><Icon name="monitor" size={13} />{t("editor.kind.rdp")}</p>
    {/if}
    <label class="field">
      <span class="label">{t("editor.name")}</span>
      <input class="input" bind:value={c.name} placeholder="web-01" />
    </label>
    <div class="grid3">
      <label class="field host">
        <span class="label">{t("editor.host")}</span>
        <input class="input mono" bind:value={c.host} placeholder="10.0.1.11" spellcheck="false" autocapitalize="off" />
      </label>
      <label class="field">
        <span class="label">{t("editor.port")}</span>
        <input class="input mono" type="number" min="1" max="65535" bind:value={c.port} />
      </label>
    </div>
    <label class="field">
      <span class="label">{t("editor.user")}</span>
      <input class="input mono" bind:value={c.user} placeholder={rdp ? "DOMAIN\\user" : "deploy"} spellcheck="false" autocapitalize="off" />
      {#if rdp}<span class="hint">{t("editor.rdpUser")}</span>{/if}
    </label>
    <label class="field">
      <span class="label">{t("editor.folder")}</span>
      <select class="select" bind:value={c.group_id}>
        <option value={null}>{t("sidebar.ungrouped")}</option>
        {#each connections.groups as g (g.id)}<option value={g.id}>{groupPath(connections.groups, g.id)}</option>{/each}
      </select>
    </label>

    <fieldset class="field">
      <legend class="label">{rdp ? t("editor.auth.password") : t("editor.auth")}</legend>
      <div class="seg" role="group" hidden={rdp}>
        {#each ["password", "key", "agent", "prompt"] as const as a (a)}
          <button type="button" aria-pressed={authType === a} onclick={() => (authType = a)}>{t(`editor.auth.${a}`)}</button>
        {/each}
      </div>
      {#if authType === "password"}
        {#if credentials.passwords.length}
          <select class="select" bind:value={passwordId} aria-label={t("editor.savedPassword")}>
            <option value="">{t("editor.newPassword")}</option>
            {#each credentials.passwords as p (p.id)}<option value={p.id}>{p.label}{others(p.id).length ? ` — ${t("editor.usedBy", { names: others(p.id).join(", ") })}` : ""}</option>{/each}
          </select>
        {/if}
        <input class="input" type="password" autocomplete="new-password" bind:value={password} placeholder={passwordId ? t("editor.passwordPlaceholder") : t("editor.auth.password")} />
        {#if !rdpPasswordOk}<span class="hint">{t("editor.rdpPassword")}</span>{/if}
      {:else if authType === "key"}
        {#if credentials.keys.length}
          <select class="select" bind:value={keyId} aria-label={t("editor.savedKey")}>
            <option value="">{t("editor.newKey")}</option>
            {#each credentials.keys as k (k.id)}<option value={k.id}>{k.label}{others(k.id).length ? ` — ${t("editor.usedBy", { names: others(k.id).join(", ") })}` : ""}</option>{/each}
          </select>
        {/if}
        {#if !keyId || keyText}
          <textarea class="textarea mono" rows="4" bind:value={keyText} placeholder="-----BEGIN OPENSSH PRIVATE KEY-----" spellcheck="false"></textarea>
        {/if}
        <div class="row-actions">
          <button type="button" class="btn sm" onclick={() => fileInput?.click()}><Icon name="key" size={13} />{keyId ? t("editor.keyPickOther") : t("editor.keyPick")}</button>
          <input type="file" class="visually-hidden" bind:this={fileInput} onchange={pickKey} tabindex="-1" />
        </div>
        <input class="input" type="password" bind:value={passphrase} placeholder={keyId && !keyText ? t("editor.passphraseKeep") : t("editor.passphrase")} autocomplete="off" />
      {:else if authType === "agent"}
        <p class="hint">{t("editor.agentNote")}</p>
      {:else}
        <p class="hint">{t("editor.promptNote")}</p>
      {/if}
    </fieldset>

    {#if rdp}
      <label class="check admin">
        <input type="checkbox" bind:checked={c.rdp_admin} />
        <span>{t("editor.rdpAdmin")}<span class="hint">{t("editor.rdpAdminHint")}</span></span>
      </label>
    {/if}

    <fieldset class="field">
      <legend class="label">{t("editor.jump")}</legend>
      {#if rdp}<p class="hint">{t("editor.rdpJump")}</p>{/if}
      {#if !jumpCandidates.length}
        <p class="hint">{t("editor.jumpNone")}</p>
      {:else}
        <div class="chips">
          {#each jumpCandidates as j (j.id)}
            <button type="button" class="chip" aria-pressed={c.jump.includes(j.id)} onclick={() => toggleJump(j.id)}>
              {#if c.jump.includes(j.id)}<span class="ord">{c.jump.indexOf(j.id) + 1}</span>{/if}{j.name}
            </button>
          {/each}
        </div>
      {/if}
    </fieldset>

    <fieldset class="field" hidden={rdp}>
      <legend class="label">{t("editor.tunnels")}</legend>
      {#each c.tunnels as x, i (i)}
        {@const problem = tunnelProblem(x)}
        <div class="tunnel" class:off={!x.enabled}>
          <div class="trow">
            <select class="select sm" bind:value={x.kind} aria-label={t("editor.tunnelKind")}>
              {#each ["local", "remote", "dynamic"] as const as k (k)}<option value={k}>{t(`editor.tunnel.${k}`)}</option>{/each}
            </select>
            <label class="on"><input type="checkbox" bind:checked={x.enabled} />{t("editor.tunnelOn")}</label>
            <span class="grow"></span>
            <button type="button" class="icon-btn" aria-label={t("editor.tunnelRemove")} title={t("editor.tunnelRemove")} onclick={() => c.tunnels.splice(i, 1)}><Icon name="trash" size={13} /></button>
          </div>
          <div class="trow">
            <span class="tl">{t(x.kind === "remote" ? "editor.tunnelListenServer" : "editor.tunnelListenHere")}</span>
            <input class="input mono sm host" class:bad={problem === "bind"} bind:value={x.bind_host} aria-label={t("editor.tunnelBindHost")} spellcheck="false" />
            <span class="colon">:</span>
            <input class="input mono sm num" class:bad={problem === "bind"} type="number" min="0" max="65535" bind:value={x.bind_port} aria-label={t("editor.tunnelBindPort")} />
          </div>
          {#if x.kind !== "dynamic"}
            <div class="trow">
              <span class="tl">{t(x.kind === "remote" ? "editor.tunnelToHere" : "editor.tunnelToServer")}</span>
              <input class="input mono sm host" class:bad={problem === "dest"} bind:value={x.dest_host} aria-label={t("editor.tunnelDestHost")} spellcheck="false" />
              <span class="colon">:</span>
              <input class="input mono sm num" class:bad={problem === "dest"} type="number" min="1" max="65535" bind:value={x.dest_port} aria-label={t("editor.tunnelDestPort")} />
            </div>
          {/if}
          <p class="hint">
            {t(`editor.tunnelHint.${x.kind}`)}
            {#if x.bind_host.trim() === "0.0.0.0" || x.bind_host.trim() === "::"}<span class="warn">{t("editor.tunnelOpen")}</span>{/if}
          </p>
        </div>
      {/each}
      <div class="row-actions">
        <button type="button" class="btn sm" onclick={() => c.tunnels.push(newTunnel())}><Icon name="plus" size={13} />{t("editor.tunnelAdd")}</button>
      </div>
      {#if c.tunnels.length}<p class="hint">{t("editor.tunnelsNext")}</p>{/if}
    </fieldset>

    <fieldset class="field">
      <legend class="label">{t("editor.env")}</legend>
      <div class="seg" role="group">
        {#each envs as e (e)}
          <button type="button" aria-pressed={c.env === e} onclick={() => (c.env = e)}>
            <span class="envdot env-{e}"></span>{t(`env.${e}`)}
          </button>
        {/each}
      </div>
    </fieldset>

    <label class="field">
      <span class="label">{t("editor.tags")}</span>
      <input class="input" bind:value={tagsText} placeholder={t("editor.tagsPlaceholder")} />
    </label>

    <label class="field" hidden={rdp}>
      <span class="label">{t("editor.approval")}</span>
      <select class="select" bind:value={c.approval_mode}>
        <option value={null}>{t("editor.approvalDefault")}</option>
        {#each ["confirm_all", "confirm_writes", "auto"] as const as m (m)}
          <option value={m as ApprovalMode} disabled={m === "auto" && c.env === "prod"}>{t(`ai.mode.${m}`)}</option>
        {/each}
      </select>
      {#if c.env === "prod"}<span class="hint">{t("editor.prodAuto")}</span>{/if}
    </label>

    <label class="field">
      <span class="label">{t("editor.notes")}</span>
      <textarea class="textarea" rows="3" bind:value={c.notes}></textarea>
    </label>
    <button type="submit" hidden aria-hidden="true"></button>
  </form>

  <footer>
    <button class="btn ghost" onclick={close}>{t("editor.cancel")}</button>
    <span class="grow"></span>
    <button class="btn" disabled={!valid || busy} onclick={() => save(true)}>{t("editor.saveConnect")}</button>
    <button class="btn primary" disabled={!valid || busy} onclick={() => save()}>{t("editor.save")}</button>
  </footer>
</div>

<style>
  .scrim {
    position: fixed;
    inset: 0;
    z-index: 40;
    background: rgba(5, 6, 8, 0.35);
    animation: fade var(--t) var(--ease);
  }
  .sheet {
    position: fixed;
    top: 0;
    right: 0;
    bottom: 0;
    z-index: 41;
    width: min(420px, 100vw);
    display: flex;
    flex-direction: column;
    background: var(--panel);
    border-left: 1px solid var(--border);
    box-shadow: var(--shadow-float);
    animation: slide var(--t) var(--ease);
  }
  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0 10px 0 18px;
    height: 48px;
    border-bottom: 1px solid var(--border);
  }
  h2 {
    margin: 0;
    font-size: 14px;
    font-weight: 600;
  }
  .body {
    flex: 1;
    overflow: auto;
    display: grid;
    align-content: start;
    gap: 16px;
    padding: 18px;
  }
  fieldset {
    border: 0;
    margin: 0;
    padding: 0;
    display: grid;
    gap: 8px;
  }
  legend {
    padding: 0;
    margin-bottom: 5px;
  }
  .grid3 {
    display: grid;
    grid-template-columns: 1fr 88px;
    gap: 10px;
  }
  .seg {
    flex-wrap: wrap;
    justify-self: start;
  }
  .seg button {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .envdot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--faint);
  }
  .envdot.env-prod {
    background: var(--env-prod);
  }
  .envdot.env-staging {
    background: var(--env-staging);
  }
  .envdot.env-dev {
    background: var(--env-dev);
  }
  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 24px;
    padding: 0 9px;
    border-radius: 999px;
    border: 1px solid var(--border-strong);
    font-size: var(--fs-meta);
    color: var(--text-2);
  }
  .chip[aria-pressed="true"] {
    border-color: var(--accent-line);
    background: var(--accent-soft);
    color: var(--accent-text);
  }
  .ord {
    font-size: 10px;
    font-weight: 600;
  }
  .row-actions {
    display: flex;
    gap: 6px;
  }
  .hint {
    margin: 0;
  }
  .tunnel {
    display: grid;
    gap: 6px;
    padding: 8px 10px;
    border: 1px solid var(--border);
    border-radius: var(--r);
  }
  .tunnel.off {
    opacity: 0.6;
  }
  .trow {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .tl {
    width: 92px;
    flex: none;
    color: var(--muted);
    font-size: var(--fs-small);
  }
  .tunnel .sm {
    height: 26px;
    font-size: 12px;
  }
  .tunnel .host {
    flex: 1;
    min-width: 0;
  }
  .tunnel .num {
    width: 72px;
    flex: none;
  }
  .tunnel .bad {
    border-color: var(--danger);
  }
  .colon {
    color: var(--faint);
  }
  .on {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font-size: var(--fs-small);
    color: var(--muted);
  }
  .on input {
    accent-color: var(--accent);
  }
  .tunnel .hint {
    font-size: var(--fs-small);
  }
  .warn {
    display: block;
    color: var(--warn);
  }
  footer {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 12px 18px;
    border-top: 1px solid var(--border);
  }
  .grow {
    flex: 1;
  }
  @keyframes slide {
    from {
      transform: translateX(16px);
      opacity: 0;
    }
  }
  @keyframes fade {
    from {
      opacity: 0;
    }
  }
  .kind button {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .kindnote {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 0;
  }
  .admin {
    align-items: flex-start;
  }
  .admin input {
    margin-top: 3px;
  }
  .admin .hint {
    display: block;
    margin-top: 2px;
  }
</style>
