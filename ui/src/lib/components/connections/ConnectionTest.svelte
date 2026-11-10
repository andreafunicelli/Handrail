<script lang="ts">
  import Icon from "../icons/Icon.svelte";
  import { t, fmtNumber } from "../../i18n";
  import { api } from "../../api";
  import { errorMessage } from "../../api/errors";
  import type { Connection, ConnTestResult, ConnTestSecrets } from "../../api/types";

  type Request = { connection: Connection; secrets: ConnTestSecrets };
  let {
    request,
    signature,
    askPassword,
    testPassword = $bindable(""),
    disabled = false,
  }: {
    /** What to test, or null when the form has nothing to log in with yet (no key). */
    request: () => Request | null;
    /** Changes whenever a field the test depends on changes: the result shown then clears. */
    signature: string;
    /** The connection asks for its password at login: offer a field for the test's own. */
    askPassword: boolean;
    testPassword?: string;
    disabled?: boolean;
  } = $props();

  type Shown = ConnTestResult | { status: "no_key"; message: null; host: null; fingerprint: null; elapsed_ms: 0 };
  let running = $state(false);
  let result = $state<{ sig: string; r: Shown } | null>(null);
  const shown = $derived(result && result.sig === signature ? result.r : null);

  const TONE: Record<Shown["status"], "ok" | "warn" | "danger"> = {
    success: "ok",
    reachable: "ok",
    auth_failed: "danger",
    unreachable: "danger",
    timeout: "warn",
    host_key_unknown: "warn",
    host_key_mismatch: "danger",
    needs_password: "warn",
    error: "danger",
    no_key: "warn",
  };
  const ICON = {
    success: "check",
    reachable: "check",
    auth_failed: "lock",
    unreachable: "wifi",
    timeout: "clock",
    host_key_unknown: "shield",
    host_key_mismatch: "shield",
    needs_password: "key",
    error: "alert",
    no_key: "key",
  } as const;

  function title(r: Shown): string {
    switch (r.status) {
      case "success":
        return t("editor.test.success", { ms: fmtNumber(r.elapsed_ms) });
      case "reachable":
        return t("editor.test.reachable");
      case "auth_failed":
        return t("editor.test.authFailed");
      case "unreachable":
        return t("editor.test.unreachable");
      case "timeout":
        return t("editor.test.timeout");
      case "host_key_unknown":
        return t("editor.test.hostKeyUnknown", { host: r.host ?? "" });
      case "host_key_mismatch":
        return t("editor.test.hostKeyMismatch");
      case "needs_password":
        return t("editor.test.needsPassword");
      case "no_key":
        return t("editor.keyPaste");
      case "error":
        return t("editor.test.error");
    }
  }

  /** A second line: what to do, or the backend's details. */
  function detail(r: Shown): string | null {
    switch (r.status) {
      case "reachable":
        return t("editor.test.reachableNote");
      case "host_key_unknown":
        return t("editor.test.hostKeyUnknownHint");
      case "host_key_mismatch":
        return t("editor.test.hostKeyMismatchHint");
      case "success":
      case "needs_password":
      case "no_key":
        return null;
      default:
        return r.message;
    }
  }

  async function test() {
    if (running || disabled) return;
    const sig = signature;
    const req = request();
    if (!req) {
      result = { sig, r: { status: "no_key", message: null, host: null, fingerprint: null, elapsed_ms: 0 } };
      return;
    }
    running = true;
    result = null;
    try {
      result = { sig, r: await api.testConnection(req.connection, req.secrets) };
    } catch (e) {
      result = { sig, r: { status: "error", message: errorMessage(e), host: null, fingerprint: null, elapsed_ms: 0 } };
    } finally {
      running = false;
    }
  }
</script>

<div class="test">
  <div class="row">
    <!-- Not disabled while running: the focus stays on it (Escape still closes the editor). -->
    <button type="button" class="btn sm" {disabled} aria-busy={running} onclick={test} aria-describedby="conn-test-result">
      {#if running}<span class="spinner" aria-hidden="true"></span>{:else}<Icon name="bolt" size={13} />{/if}
      {t("editor.test")}
    </button>
    {#if askPassword}
      <input
        class="input sm pw"
        type="password"
        autocomplete="off"
        bind:value={testPassword}
        placeholder={t("editor.test.password")}
        aria-label={t("editor.test.password")}
        onkeydown={(e) => {
          if (e.key === "Enter" && !e.metaKey && !e.ctrlKey) {
            e.preventDefault();
            test();
          }
        }}
      />
    {/if}
  </div>
  <div id="conn-test-result" class="result" aria-live="polite">
    {#if running}
      <p class="line muted">{t("editor.test.running")}</p>
    {:else if shown}
      {@const d = detail(shown)}
      <p class="line {TONE[shown.status]}">
        <Icon name={ICON[shown.status]} size={13} />
        <span class="txt">
          <span class="ttl">{title(shown)}</span>
          {#if shown.status === "host_key_unknown" && shown.fingerprint}<span class="fp mono">{shown.fingerprint}</span>{/if}
          {#if d}<span class="dtl">{d}</span>{/if}
        </span>
      </p>
    {/if}
  </div>
</div>

<style>
  .test {
    display: grid;
    gap: var(--sp-2);
  }
  .row {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
  }
  .row .btn {
    flex: none;
    gap: 6px;
  }
  .pw {
    flex: 1;
    min-width: 0;
    height: var(--ctl-h-sm);
    font-size: var(--fs-meta);
  }
  .result:empty {
    display: none;
  }
  .line {
    display: flex;
    align-items: flex-start;
    gap: 7px;
    margin: 0;
    font-size: var(--fs-meta);
    line-height: var(--lh);
  }
  .line > :global(svg) {
    flex: none;
    margin-top: 0.2em;
  }
  .txt {
    display: grid;
    gap: 2px;
    min-width: 0;
  }
  .ttl {
    font-weight: 500;
  }
  .dtl {
    color: var(--muted);
    font-size: var(--fs-small);
    overflow-wrap: anywhere;
  }
  .fp {
    color: var(--text-2);
    font-size: var(--fs-small);
    overflow-wrap: anywhere;
  }
  .muted {
    color: var(--muted);
  }
  .ok {
    color: var(--ok);
  }
  .warn {
    color: var(--warn);
  }
  .danger {
    color: var(--danger);
  }
  .spinner {
    width: 11px;
    height: 11px;
    border-radius: 50%;
    border: 1.5px solid currentColor;
    border-right-color: transparent;
    animation: spin 0.8s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .spinner {
      animation-duration: 2.4s;
    }
  }
</style>
