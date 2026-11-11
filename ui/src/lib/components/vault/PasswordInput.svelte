<script lang="ts">
  import Icon from "../icons/Icon.svelte";
  import { t } from "../../i18n";

  /** A master-password field with a show/hide toggle. Hidden by default; the screen hides it
   *  again when the form is submitted (`shown` is bindable so related fields share one state). */
  let {
    value = $bindable(""),
    shown = $bindable(false),
    autocomplete,
    invalid = false,
    autofocus = false,
  }: { value?: string; shown?: boolean; autocomplete: "current-password" | "new-password"; invalid?: boolean; autofocus?: boolean } = $props();
</script>

<span class="pw">
  <input
    class="input lg"
    type={shown ? "text" : "password"}
    {autocomplete}
    autocapitalize="off"
    spellcheck="false"
    bind:value
    aria-invalid={invalid}
    data-autofocus={autofocus || undefined}
  />
  <button
    type="button"
    class="toggle"
    aria-label={shown ? t("vault.hidePassword") : t("vault.showPassword")}
    title={shown ? t("vault.hidePassword") : t("vault.showPassword")}
    aria-pressed={shown}
    onclick={() => (shown = !shown)}
  >
    <Icon name={shown ? "eyeOff" : "eye"} size={16} />
  </button>
</span>

<style>
  .pw {
    position: relative;
    display: block;
  }
  .pw input {
    width: 100%;
    height: 36px;
    font-size: var(--fs-heading);
    padding-right: 40px;
  }
  .toggle {
    position: absolute;
    top: 50%;
    right: 6px;
    transform: translateY(-50%);
    display: grid;
    place-items: center;
    width: 28px;
    height: 28px;
    padding: 0;
    border: 0;
    border-radius: var(--radius-sm, 6px);
    background: transparent;
    color: var(--muted);
    cursor: pointer;
  }
  .toggle:hover,
  .toggle:focus-visible {
    color: var(--fg);
    background: var(--hover, rgba(127, 127, 127, 0.12));
  }
</style>
