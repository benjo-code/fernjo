import { flushSync, mount, unmount } from "svelte";
import { afterEach, describe, expect, test } from "vitest";
import { TreeStore } from "../store.svelte";
import { mockApi } from "../testing";
import FilterBox from "./FilterBox.svelte";

let mounted: ReturnType<typeof mount> | null = null;

afterEach(() => {
  if (mounted) unmount(mounted);
  mounted = null;
  document.body.innerHTML = "";
});

function setup(mac?: boolean) {
  const store = new TreeStore(mockApi([]).api);
  mounted = mount(FilterBox, {
    target: document.body,
    props: { store, ...(mac === undefined ? {} : { mac }) },
  });
  flushSync();
  return document.querySelector<HTMLInputElement>("input")!;
}

describe("FilterBox", () => {
  test("the_short_placeholder_keeps_the_full_accessible_name", () => {
    const input = setup(false);
    expect(input.placeholder).toBe("Filter");
    expect(input.getAttribute("aria-label")).toBe("Filter connections");
  });

  test("the_tooltip_names_the_platform_correct_chord", () => {
    expect(setup(false).title).toBe("Filter connections (Ctrl+Shift+K)");
    unmount(mounted!);
    mounted = null;
    document.body.innerHTML = "";
    expect(setup(true).title).toBe("Filter connections (Cmd+K)");
  });
});
