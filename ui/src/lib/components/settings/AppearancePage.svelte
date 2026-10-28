<script lang="ts">
  import { segmented } from "../../actions/segmented";
  import { t } from "../../i18n";
  import { settings, type TerminalEditorChoice, type Theme } from "../../stores/settings.svelte";
  import { SCHEMES } from "../../terminal/schemes";

  const themes: Theme[] = ["dark", "light", "system"];
  const editorChoices: TerminalEditorChoice[] = ["ask", "editor", "terminal"];
</script>

<div class="field">
  <span class="label">{t("app.theme")}</span>
  <div class="seg" role="group" use:segmented aria-label={t("app.theme")}>
    {#each themes as th (th)}
      <button aria-pressed={settings.theme === th} onclick={() => (settings.theme = th)}>{t(`app.theme.${th}`)}</button>
    {/each}
  </div>
</div>

<h3>{t("app.fontSize")}</h3>
<div class="size">
  <input type="range" min="10" max="20" step="1" bind:value={settings.fontSize} aria-label={t("app.fontSize")} />
  <span class="mono val">{settings.fontSize}px</span>
</div>

<label class="check">
  <input type="checkbox" bind:checked={settings.sessionBar} />
  <span>{t("app.sessionBar")}<span class="hint">{t("app.sessionBarHint")}</span></span>
</label>

<div class="field">
  <span class="label">{t("app.terminalEditor")}</span>
  <div class="seg" role="group" use:segmented aria-label={t("app.terminalEditor")}>
    {#each editorChoices as c (c)}
      <button aria-pressed={settings.terminalEditor === c} onclick={() => (settings.terminalEditor = c)}>{t(`app.terminalEditor.${c}`)}</button>
    {/each}
  </div>
  <span class="hint">{t("app.terminalEditorHint")}</span>
</div>

<h3>{t("app.scheme")}</h3>
<div class="schemes" role="radiogroup" aria-label={t("app.scheme")}>
  {#each SCHEMES as s (s.id)}
    <button
      class="scheme"
      role="radio"
      aria-checked={settings.scheme === s.id}
      onclick={() => (settings.scheme = s.id)}
      style:background={s.theme.background}
      style:color={s.theme.foreground}
    >
      <span class="prompt mono"><span style:color={s.theme.blue}>dev@web-01</span>:<span style:color={s.theme.brightBlack}>~</span>$ ls</span>
      <span class="swatches">
        {#each [s.theme.red, s.theme.green, s.theme.yellow, s.theme.blue, s.theme.magenta, s.theme.cyan] as c, i (i)}<span style:background={c}></span>{/each}
      </span>
      <span class="sname">{s.name}</span>
    </button>
  {/each}
</div>

<style>
  .seg {
    justify-self: start;
  }
  .size {
    display: flex;
    align-items: center;
    gap: 12px;
    max-width: 360px;
  }
  .size input {
    flex: 1;
    accent-color: var(--accent);
  }
  .val {
    font-size: 12px;
    color: var(--muted);
    min-width: 40px;
  }
  .check {
    display: flex;
    align-items: flex-start;
    gap: 9px;
    margin-top: 16px;
    max-width: 520px;
  }
  .check input {
    margin-top: 3px;
    accent-color: var(--accent);
  }
  .check .hint {
    display: block;
    margin-top: 2px;
  }
  .schemes {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(190px, 1fr));
    gap: 10px;
  }
  .scheme {
    display: grid;
    gap: 10px;
    padding: 12px;
    border-radius: var(--r-lg);
    text-align: left;
    box-shadow: 0 0 0 1px var(--border);
    transition: box-shadow var(--t-fast) var(--ease);
  }
  .scheme[aria-checked="true"] {
    box-shadow: 0 0 0 2px var(--accent);
  }
  .prompt {
    font-size: 11.5px;
  }
  .swatches {
    display: flex;
    gap: 4px;
  }
  .swatches span {
    width: 14px;
    height: 6px;
    border-radius: 2px;
  }
  .sname {
    font-size: var(--fs-meta);
    opacity: 0.8;
  }
</style>
