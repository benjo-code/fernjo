import "@fontsource-variable/hanken-grotesk/wght.css";
import "@fontsource-variable/red-hat-mono/wght.css";
import { mount } from "svelte";
import "./styles/tokens.css";
import "./styles/base.css";
import App from "./App.svelte";
import SpikePage from "./spike/SpikePage.svelte";
import { applyTheme, readStoredTheme } from "./lib/theme.svelte";

const target = document.getElementById("app");
if (!target) {
  throw new Error("Missing #app mount point");
}

// before mount, so an explicit choice never flashes the other palette
applyTheme(readStoredTheme());

// throwaway spike (branch spike/m08-rdp-canvas): open the app with the URL hash #spike
// Ctrl+Shift+F12 switches the window to the spike page (a release build has no address bar)
window.addEventListener("keydown", (e) => {
  if (e.ctrlKey && e.shiftKey && e.key === "F12") {
    location.hash = "#spike";
    location.reload();
  }
});

if (location.hash === "#spike") {
  mount(SpikePage, { target });
} else {
  mount(App, { target });
}
