import "@fontsource-variable/hanken-grotesk/wght.css";
import "@fontsource-variable/red-hat-mono/wght.css";
import { mount } from "svelte";
import "./styles/tokens.css";
import "./styles/base.css";
import App from "./App.svelte";
import { applyTheme, readStoredTheme } from "./lib/theme.svelte";

const target = document.getElementById("app");
if (!target) {
  throw new Error("Missing #app mount point");
}

// before mount, so an explicit choice never flashes the other palette
applyTheme(readStoredTheme());

mount(App, { target });
