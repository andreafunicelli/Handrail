<script lang="ts">
  import ConnIcon from "../common/ConnIcon.svelte";
  import { isIconName } from "../icons/Icon.svelte";
  import { CONNECTION_ICONS, singleGlyph } from "../../util/connIcon";
  import { segmented } from "../../actions/segmented";
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
  import ConnectionTest from "./ConnectionTest.svelte";
  import JumpHostPicker from "./JumpHostPicker.svelte";
  import { initialSections, networkHasError, testLogin, testSignature, type LoginForm } from "./editorForm";

  let { connection, isNew }: { connection: Connection; isNew: boolean } = $props();

  // Local editable copy (the prop is already a detached clone made by ui.edit).
  // svelte-ignore state_referenced_locally
  let c = $state({ ...connection, tunnels: connection.tunnels ?? [] });
  let iconOpen = $state(false);
  /** What is typed in the emoji field. It, not the last valid glyph, decides the icon: cleared
   *  means no icon, more than one glyph blocks saving until fixed. */
  // svelte-ignore state_referenced_locally
  let emojiDraft = $state(c.icon && !isIconName(c.icon) ? c.icon : "");
  const emojiBad = $derived(emojiDraft.trim() !== "" && !singleGlyph(emojiDraft));
  function setIcon(icon: string | null) {
    c.icon = icon;
    emojiDraft = "";
    iconOpen = false;
  }
  function typeEmoji(value: string) {
    emojiDraft = value;
    const g = singleGlyph(value);
    if (g) c.icon = g;
    else if (!value.trim() && c.icon && !isIconName(c.icon)) c.icon = null;
  }
  let authType = $state<"password" | "key" | "agent" | "prompt">(c.auth.type);
  /** Log in as this saved identity instead of the connection's own user and login. */
  let identityId = $state(c.identity ?? "");
  const identity = $derived(credentials.identity(identityId));
  /** RDP needs a saved password, so it offers only identities that have one. */
  const identityChoices = $derived(credentials.identities.filter((i) => c.kind === "ssh" || i.auth.type === "password"));
  let password = $state("");
  let keyText = $state("");
  let passphrase = $state("");
  let tagsText = $state(c.tags.join(", "));
  /** Network and Advanced start open when they hold something (or an error, see below). */
  // svelte-ignore state_referenced_locally
  const opened = initialSections(c, tagsText);
  let networkOpen = $state(opened.network);
  let advancedOpen = $state(opened.advanced);
  /** Typed for a test of a connection that asks for its password; never saved. */
  let testPassword = $state("");
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
  /** Connections reaching others through this one: it has to stay SSH for them. */
  const jumpFor = $derived(connections.list.filter((x) => x.id !== c.id && x.jump.includes(c.id)).map((x) => x.name));
  const tunnelsOk = $derived(c.tunnels.every((x) => !tunnelProblem(x)));
  const rdp = $derived(c.kind === "rdp");
  /** MySQL/MariaDB or PostgreSQL: a password (saved or asked), a database, encryption. */
  const dbKind = $derived(c.kind === "mysql" || c.kind === "postgres");
  /** Databases log in with a password (saved or asked at connect time). */
  const authKinds = $derived(dbKind ? (["password", "prompt"] as const) : (["password", "key", "agent", "prompt"] as const));
  /** RDP logs in with a saved password only (NLA needs it before the session starts). */
  const rdpPasswordOk = $derived(!rdp || (identityId ? identity?.auth.type === "password" : !!password || !!passwordId));
  const valid = $derived(c.name.trim() !== "" && c.host.trim() !== "" && c.port > 0 && c.port < 65536 && tunnelsOk && rdpPasswordOk && !emojiBad);

  // A field that keeps the connection from being saved is never hidden in a closed section.
  $effect(() => {
    if (networkHasError(c)) networkOpen = true;
  });
  $effect(() => {
    if (emojiBad) advancedOpen = true;
  });

  /** One line under a closed section's title: what it holds. */
  const networkSummary = $derived.by(() => {
    const parts: string[] = [];
    const names = c.jump.map((id) => connections.list.find((x) => x.id === id)?.name).filter(Boolean);
    parts.push(names.length ? t("editor.sum.via", { names: names.join(" → ") }) : t("editor.jumpNone"));
    if (c.kind === "ssh" && c.tunnels.length) parts.push(t("editor.sum.forwards", { n: c.tunnels.length }));
    if (rdp && c.rdp_admin) parts.push("/admin");
    if (dbKind) parts.push(t(`editor.dbTls.${c.db_tls ?? "prefer"}`));
    return parts.join(" · ");
  });
  const advancedSummary = $derived.by(() => {
    const parts: string[] = [];
    const tags = tagsText.split(",").map((x) => x.trim()).filter(Boolean);
    if (tags.length) parts.push(tags.join(", "));
    if (!rdp && c.approval_mode) parts.push(t(`ai.mode.${c.approval_mode}`));
    if (c.notes.trim()) parts.push(t("editor.notes"));
    if (c.icon) parts.push(t("editor.icon"));
    return parts.length ? parts.join(" · ") : t("editor.sum.none");
  });

  // ---- connection test (of the form as it is; nothing is saved)
  const login: LoginForm = $derived({
    authType,
    identityId,
    password,
    passwordId,
    keyText,
    keyId,
    keyPassphraseId: keyId ? (c.auth.type === "key" && c.auth.key === keyId ? keptPassphrase : credentials.passphraseFor(keyId)) : null,
    passphrase,
    testPassword,
  });
  const testSig = $derived(testSignature(c, login));
  const testable = $derived(c.host.trim() !== "" && Number.isInteger(c.port) && c.port > 0 && c.port < 65536);
  /** Logs in with a password asked at connect time: the test needs one typed for it. */
  const testAsks = $derived(!identityId && !rdp && (authType === "prompt" || (authType === "password" && !password && !passwordId)));
  function testRequest() {
    const l = testLogin(login);
    if (!l) return null;
    const out = $state.snapshot(c) as Connection;
    out.host = out.host.trim();
    out.user = identity ? identity.user : out.user.trim();
    out.auth = l.auth;
    out.identity = l.identity;
    // A test starts no forward (and half-typed ones would not be valid).
    out.tunnels = [];
    out.database = dbKind ? out.database?.trim() || null : null;
    out.db_tls = dbKind ? (out.db_tls ?? null) : null;
    return { connection: out, secrets: l.secrets };
  }

  const DEFAULT_PORTS: Record<string, number> = { ssh: 22, rdp: 3389, mysql: 3306, postgres: 5432 };

  function setKind(kind: "ssh" | "rdp" | "mysql" | "postgres") {
    if (c.kind === kind) return;
    // A port left at the old kind's default follows the new kind.
    if (c.port === DEFAULT_PORTS[c.kind]) c.port = DEFAULT_PORTS[kind];
    if (kind === "rdp") authType = "password";
    if ((kind === "mysql" || kind === "postgres") && (authType === "key" || authType === "agent")) authType = "password";
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
      out.icon = out.icon || null;
      out.host = out.host.trim();
      out.user = out.user.trim();
      out.tags = tagsText.split(",").map((s) => s.trim()).filter(Boolean);
      out.tunnels = out.tunnels.map((x) => ({ ...x, bind_host: x.bind_host.trim(), dest_host: x.kind === "dynamic" ? "" : x.dest_host.trim(), dest_port: x.kind === "dynamic" ? 0 : x.dest_port }));
      if (out.env === "prod" && out.approval_mode === "auto") out.approval_mode = null;
      if (out.kind !== "ssh") out.tunnels = [];
      if (out.kind === "rdp") {
        // Agent approvals belong to terminal and database sessions.
        out.approval_mode = null;
      } else {
        out.rdp_admin = false;
      }
      if (out.kind === "mysql" || out.kind === "postgres") {
        out.database = out.database?.trim() || null;
        out.db_tls = out.db_tls ?? null;
      } else {
        out.database = null;
        out.db_tls = null;
      }
      out.identity = identityId || null;
      if (identity) {
        // The identity's user and login apply when connecting; keep the user for lists.
        out.user = identity.user;
        out.auth = { type: "prompt" };
      } else if (authType === "password") {
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
      password = keyText = passphrase = testPassword = "";
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
    <!-- GENERAL: what and where -->
    <section class="sec" aria-labelledby="sec-general">
      <h3 class="sec-title" id="sec-general">{t("editor.sec.general")}</h3>
      <div class="seg kind" role="group" use:segmented aria-label={t("editor.kind")}>
        <button type="button" aria-pressed={c.kind === "ssh"} onclick={() => setKind("ssh")}><Icon name="terminal" size={13} />SSH</button>
        {#each [["rdp", "monitor", t("editor.kind.rdp")], ["mysql", "database", "MySQL"], ["postgres", "database", "PostgreSQL"]] as const as [k, icon, label] (k)}
          <button
            type="button"
            aria-pressed={c.kind === k}
            disabled={c.kind !== k && jumpFor.length > 0}
            title={c.kind !== k && jumpFor.length ? t("editor.kind.rdpJump", { names: jumpFor.join(", ") }) : undefined}
            onclick={() => setKind(k)}><Icon name={icon} size={13} />{label}</button
          >
        {/each}
      </div>
      {#if c.kind === "ssh" && c.port === 3389 && !jumpFor.length}
        <p class="hint kindnote">
          <Icon name="monitor" size={13} />{t("editor.kind.rdpPort")}
          <button type="button" class="btn sm" onclick={() => setKind("rdp")}>{t("editor.kind.toRdp")}</button>
        </p>
      {/if}
      <label class="field">
        <span class="label">{t("editor.name")}</span>
        <input class="input" bind:value={c.name} placeholder="web-01" />
      </label>
      <div class="grid3">
        <label class="field host">
          <span class="label">{t("editor.host")}</span>
          <input class="input mono" bind:value={c.host} placeholder={dbKind && c.jump.length ? "localhost" : "10.0.1.11"} spellcheck="false" autocapitalize="off" />
        </label>
        <label class="field">
          <span class="label">{t("editor.port")}</span>
          <input class="input mono" type="number" min="1" max="65535" bind:value={c.port} />
        </label>
      </div>
      {#if identityId}
        <label class="field">
          <span class="label">{t("editor.user")}</span>
          <input class="input mono" value={identity?.user ?? ""} disabled />
          <span class="hint">{t("editor.userFromIdentity")}</span>
        </label>
      {:else}
        <label class="field">
          <span class="label">{t("editor.user")}</span>
          <input class="input mono" bind:value={c.user} placeholder={rdp ? "DOMAIN\\user" : dbKind ? "app" : "deploy"} spellcheck="false" autocapitalize="off" />
          {#if rdp}<span class="hint">{t("editor.rdpUser")}</span>{/if}
        </label>
      {/if}
      {#if dbKind}
        <label class="field">
          <span class="label">{t("editor.database")}</span>
          <input class="input mono" bind:value={c.database} placeholder={c.kind === "postgres" ? "postgres" : t("editor.databaseAny")} spellcheck="false" autocapitalize="off" />
          {#if c.kind === "postgres"}<span class="hint">{t("editor.databasePg")}</span>{/if}
        </label>
      {/if}
      <label class="field">
        <span class="label">{t("editor.folder")}</span>
        <select class="select" bind:value={c.group_id}>
          <option value={null}>{t("sidebar.ungrouped")}</option>
          {#each connections.groups as g (g.id)}<option value={g.id}>{groupPath(connections.groups, g.id)}</option>{/each}
        </select>
      </label>
      <fieldset class="field">
        <legend class="label">{t("editor.env")}</legend>
        <div class="seg" role="group" use:segmented>
          {#each envs as e (e)}
            <button type="button" aria-pressed={c.env === e} onclick={() => (c.env = e)}>
              <span class="envdot env-{e}"></span>{t(`env.${e}`)}
            </button>
          {/each}
        </div>
      </fieldset>
    </section>

    <!-- AUTHENTICATION: who logs in, and how -->
    <section class="sec" aria-labelledby="sec-auth">
      <h3 class="sec-title" id="sec-auth">{t("editor.auth")}</h3>
      {#if identityChoices.length || identityId}
        <label class="field">
          <span class="label">{t("editor.loginAs")}</span>
          <select class="select" bind:value={identityId}>
            <option value="">{t("editor.ownLogin")}</option>
            {#each identityChoices as i (i.id)}<option value={i.id}>{i.label} — {i.user}</option>{/each}
            {#if identityId && !identity}<option value={identityId}>{t("editor.identityMissing")}</option>{/if}
          </select>
          {#if identity}<span class="hint">{t("editor.identityNote", { user: identity.user })}</span>{/if}
          {#if identityId && !identity}<span class="hint">{t("editor.identityMissingHint")}</span>{/if}
        </label>
      {/if}
      {#if !identityId}
        <fieldset class="field">
          <legend class="label">{rdp ? t("editor.auth.password") : t("editor.auth.method")}</legend>
          <div class="seg" role="group" hidden={rdp} use:segmented>
            {#each authKinds as a (a)}
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
            <input
              class="input"
              type="password"
              autocomplete="new-password"
              bind:value={password}
              placeholder={passwordId ? t("editor.passwordPlaceholder") : t("editor.auth.password")}
              aria-label={t("editor.auth.password")}
            />
            {#if !rdpPasswordOk}<span class="hint">{t("editor.rdpPassword")}</span>{/if}
          {:else if authType === "key"}
            {#if credentials.keys.length}
              <select class="select" bind:value={keyId} aria-label={t("editor.savedKey")}>
                <option value="">{t("editor.newKey")}</option>
                {#each credentials.keys as k (k.id)}<option value={k.id}>{k.label}{others(k.id).length ? ` — ${t("editor.usedBy", { names: others(k.id).join(", ") })}` : ""}</option>{/each}
              </select>
            {/if}
            {#if !keyId || keyText}
              <textarea class="textarea mono" rows="4" bind:value={keyText} placeholder="-----BEGIN OPENSSH PRIVATE KEY-----" spellcheck="false" aria-label={t("editor.auth.key")}></textarea>
            {/if}
            <div class="row-actions">
              <button type="button" class="btn sm" onclick={() => fileInput?.click()}><Icon name="key" size={13} />{keyId ? t("editor.keyPickOther") : t("editor.keyPick")}</button>
              <input type="file" class="visually-hidden" bind:this={fileInput} onchange={pickKey} tabindex="-1" />
            </div>
            <input
              class="input"
              type="password"
              bind:value={passphrase}
              placeholder={keyId && !keyText ? t("editor.passphraseKeep") : t("editor.passphrase")}
              aria-label={t("editor.passphrase")}
              autocomplete="off"
            />
          {:else if authType === "agent"}
            <p class="hint">{t("editor.agentNote")}</p>
          {:else}
            <p class="hint">{t("editor.promptNote")}</p>
          {/if}
        </fieldset>
      {/if}
    </section>

    <!-- NETWORK: how it is reached (collapsible) -->
    <section class="sec">
      <h3 class="sec-head">
        <button type="button" class="sec-toggle" aria-expanded={networkOpen} aria-controls="sec-network" onclick={() => (networkOpen = !networkOpen)}>
          <Icon name={networkOpen ? "chevronDown" : "chevronRight"} size={12} />
          <span class="sec-name">{t("editor.sec.network")}</span>
          {#if !networkOpen}<span class="sec-sum">{networkSummary}</span>{/if}
        </button>
      </h3>
      <div class="sec-body" id="sec-network" hidden={!networkOpen}>
        <fieldset class="field">
          <legend class="label">{t(dbKind ? "editor.dbTunnel" : "editor.jump")}</legend>
          {#if rdp}<p class="hint">{t("editor.rdpJump")}</p>{/if}
          {#if dbKind}<p class="hint">{t("editor.dbTunnelHint")}</p>{/if}
          <JumpHostPicker candidates={jumpCandidates} groups={connections.groups} bind:value={c.jump} label={t(dbKind ? "editor.dbTunnel" : "editor.jump")} />
        </fieldset>

        {#if dbKind}
          <label class="field">
            <span class="label">{t("editor.dbTls")}</span>
            <select class="select" bind:value={c.db_tls}>
              <option value={null}>{t("editor.dbTls.prefer")}</option>
              <option value="require">{t("editor.dbTls.require")}</option>
              <option value="verify">{t("editor.dbTls.verify")}</option>
              <option value="disable">{t("editor.dbTls.disable")}</option>
            </select>
          </label>
        {/if}

        {#if rdp}
          <label class="check admin">
            <input type="checkbox" bind:checked={c.rdp_admin} />
            <span>{t("editor.rdpAdmin")}<span class="hint">{t("editor.rdpAdminHint")}</span></span>
          </label>
        {/if}

        <fieldset class="field" hidden={c.kind !== "ssh"}>
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
                <input class="input mono sm host" class:bad={problem === "bind"} bind:value={x.bind_host} aria-label={t("editor.tunnelBindHost")} aria-invalid={problem === "bind"} spellcheck="false" />
                <span class="colon">:</span>
                <input class="input mono sm num" class:bad={problem === "bind"} type="number" min="0" max="65535" bind:value={x.bind_port} aria-label={t("editor.tunnelBindPort")} />
              </div>
              {#if x.kind !== "dynamic"}
                <div class="trow">
                  <span class="tl">{t(x.kind === "remote" ? "editor.tunnelToHere" : "editor.tunnelToServer")}</span>
                  <input class="input mono sm host" class:bad={problem === "dest"} bind:value={x.dest_host} aria-label={t("editor.tunnelDestHost")} aria-invalid={problem === "dest"} spellcheck="false" />
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
      </div>
    </section>

    <!-- ADVANCED: labels, agents, notes (collapsible) -->
    <section class="sec">
      <h3 class="sec-head">
        <button type="button" class="sec-toggle" aria-expanded={advancedOpen} aria-controls="sec-advanced" onclick={() => (advancedOpen = !advancedOpen)}>
          <Icon name={advancedOpen ? "chevronDown" : "chevronRight"} size={12} />
          <span class="sec-name">{t("editor.sec.advanced")}</span>
          {#if !advancedOpen}<span class="sec-sum">{advancedSummary}</span>{/if}
        </button>
      </h3>
      <div class="sec-body" id="sec-advanced" hidden={!advancedOpen}>
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

        <div class="field">
          <span class="label" id="conn-icon-label">{t("editor.icon")}</span>
          <div class="iconrow">
            <button
              type="button"
              class="icon-pick"
              title={t("editor.icon")}
              aria-labelledby="conn-icon-label"
              aria-expanded={iconOpen}
              onclick={() => (iconOpen = !iconOpen || emojiBad)}
            >
              {#if c.icon}<ConnIcon icon={c.icon} size={15} />{:else}<Icon name="plus" size={13} />{/if}
            </button>
            <span class="hint">{c.icon ? (isIconName(c.icon) ? c.icon : t("editor.iconEmojiSet")) : t("editor.iconNone")}</span>
          </div>
          {#if iconOpen}
            <div class="icon-grid" role="group" aria-label={t("editor.icon")}>
              <button type="button" aria-pressed={!c.icon} title={t("editor.iconNone")} aria-label={t("editor.iconNone")} onclick={() => setIcon(null)}>
                <Icon name="x" size={13} />
              </button>
              {#each CONNECTION_ICONS as n (n)}
                <button type="button" aria-pressed={c.icon === n} title={n} aria-label={n} onclick={() => setIcon(n)}><Icon name={n} size={15} /></button>
              {/each}
              <input
                class="input emoji-in"
                placeholder={t("editor.iconEmoji")}
                aria-label={t("editor.iconEmoji")}
                aria-invalid={emojiBad}
                aria-describedby={emojiBad ? "emoji-err" : undefined}
                value={emojiDraft}
                oninput={(e) => typeEmoji(e.currentTarget.value)}
              />
              {#if emojiBad}<span class="err emoji-in" id="emoji-err">{t("editor.iconOne")}</span>{/if}
            </div>
          {/if}
        </div>

        <label class="field">
          <span class="label">{t("editor.notes")}</span>
          <textarea class="textarea" rows="3" bind:value={c.notes}></textarea>
        </label>
      </div>
    </section>
    <button type="submit" hidden aria-hidden="true"></button>
  </form>

  <div class="testbar">
    <ConnectionTest request={testRequest} signature={testSig} askPassword={testAsks} bind:testPassword disabled={!testable || busy} />
  </div>

  <footer>
    <button class="btn ghost" onclick={close}>{t("editor.cancel")}</button>
    <span class="grow"></span>
    <button class="btn" disabled={!valid || busy} onclick={() => save(true)}>{t("editor.saveConnect")}</button>
    <button class="btn primary" disabled={!valid || busy} onclick={() => save()}>{t("editor.save")}</button>
  </footer>
</div>

<style>
  .iconrow {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
  }
  .icon-pick {
    display: inline-grid;
    place-items: center;
    flex: none;
    width: var(--ctl-h);
    height: var(--ctl-h);
    border-radius: var(--r);
    border: 1px solid var(--border-strong);
    background: var(--bg);
    color: var(--muted);
  }
  .icon-pick:hover,
  .icon-pick[aria-expanded="true"] {
    border-color: var(--accent-line);
    color: var(--text);
  }
  .icon-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(30px, 1fr));
    gap: 4px;
    padding: 6px;
    border: 1px solid var(--border);
    border-radius: var(--r);
    background: var(--bg);
  }
  .icon-grid button {
    display: grid;
    place-items: center;
    height: 30px;
    border-radius: var(--r-sm);
    color: var(--text-2);
  }
  .icon-grid button:hover {
    background: var(--hover);
    color: var(--text);
  }
  .icon-grid button[aria-pressed="true"] {
    background: var(--accent-soft);
    color: var(--accent-text);
    box-shadow: inset 0 0 0 1px var(--accent-line);
  }
  .emoji-in {
    grid-column: 1 / -1;
  }
  .err {
    color: var(--danger);
    font-size: var(--fs-meta);
  }
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
    width: min(480px, 100vw);
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
    font-size: var(--fs-heading);
    font-weight: 600;
  }
  .body {
    flex: 1;
    overflow: auto;
    display: grid;
    align-content: start;
    gap: var(--sp-4);
    padding: var(--sp-4) 18px var(--sp-5);
  }
  /* Sections: General and Authentication always open; Network and Advanced collapse. */
  .sec {
    display: grid;
    gap: var(--sp-3);
    min-width: 0;
  }
  .sec + .sec {
    padding-top: var(--sp-4);
    border-top: 1px solid var(--border);
  }
  .sec-body {
    display: grid;
    gap: var(--sp-3);
    min-width: 0;
  }
  .sec-body[hidden] {
    display: none;
  }
  .sec-title,
  .sec-head {
    margin: 0;
    font-size: var(--fs-small);
    font-weight: 600;
    letter-spacing: 0.05em;
    text-transform: uppercase;
    color: var(--muted);
  }
  .sec-toggle {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    min-height: var(--ctl-h-sm);
    margin: calc(var(--sp-1) * -1) 0;
    border-radius: var(--r-sm);
    color: inherit;
    font: inherit;
    letter-spacing: inherit;
    text-transform: inherit;
    text-align: left;
  }
  .sec-toggle:hover {
    color: var(--text-2);
  }
  .sec-name {
    flex: none;
  }
  .sec-sum {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    text-align: right;
    font-weight: 400;
    letter-spacing: 0;
    text-transform: none;
    font-size: var(--fs-meta);
    color: var(--meta);
  }
  .testbar {
    padding: var(--sp-3) 18px;
    border-top: 1px solid var(--border);
    background: var(--panel);
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
    gap: var(--sp-3);
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
    font-size: var(--fs-meta);
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
    gap: var(--sp-2);
    padding: var(--sp-3) 18px;
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
