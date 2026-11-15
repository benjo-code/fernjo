import { mount, unmount } from "svelte";
import { afterEach, expect, test } from "vitest";
import App from "./App.svelte";

afterEach(() => {
  document.body.innerHTML = "";
});

test("renders the app title", () => {
  const app = mount(App, { target: document.body });
  expect(document.querySelector("h1")?.textContent).toBe("fernjo");
  unmount(app);
});
