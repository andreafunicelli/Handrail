<script lang="ts">
  import { onMount } from "svelte";
  import { t } from "../../i18n";
  import { invoke } from "@tauri-apps/api/core";
  import { api, isTauri } from "../../api";

  let version = $state("0.1.0");
  onMount(async () => {
    if (!isTauri) return;
    try {
      version = await invoke<string>("app_version");
    } catch {
      /* keep default */
    }
  });
</script>

<div class="about">
  <div class="mark" aria-hidden="true">✦</div>
  <div>
    <strong>{t("app.name")}</strong>
    <p class="intro">{t("about.body")}</p>
  </div>
</div>
<dl>
  <dt>{t("about.version")}</dt>
  <dd class="mono">{version}</dd>
  <dt>{t("about.backend")}</dt>
  <dd>{api.backend === "mock" ? t("about.backend.mock") : t("about.backend.tauri")}</dd>
</dl>

<style>
  .about {
    display: flex;
    gap: 14px;
    align-items: flex-start;
  }
  .mark {
    display: grid;
    place-items: center;
    width: 40px;
    height: 40px;
    border-radius: 10px;
    border: 1px solid var(--border-strong);
    color: var(--accent);
    flex: none;
  }
  strong {
    font-size: 15px;
    font-weight: 600;
  }
  .intro {
    margin: 2px 0 0 !important;
  }
  dl {
    display: grid;
    grid-template-columns: 120px 1fr;
    gap: 8px;
    margin: 24px 0 0;
  }
  dt {
    color: var(--muted);
  }
  dd {
    margin: 0;
  }
</style>
