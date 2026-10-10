<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  let version = $state("…");
  invoke<string>("app_version").then((v) => (version = v)).catch(() => (version = "web"));
</script>

<main>AIHubTerm {version}</main>
