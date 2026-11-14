import { mount } from "svelte";
import "@xterm/xterm/css/xterm.css";
import "./app.css";
import App from "./App.svelte";
import { isTauri } from "./lib/api";
import { t } from "./lib/i18n";

const target = document.getElementById("app")!;
// A production build has no mock backend: outside the desktop app there is nothing to talk to.
if (isTauri || import.meta.env.DEV) mount(App, { target });
else target.textContent = t("app.noBackend");
