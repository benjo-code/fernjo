import { describe, expect, test } from "vitest";
import { apiError, credential, mockApi, vaultStatus } from "./testing";
import { VaultStore } from "./vault.svelte";

const locked = () => vaultStatus({ state: "locked", path: "/d/vault.fernjo" });
const unlocked = () => vaultStatus({ state: "unlocked", path: "/d/vault.fernjo" });

describe("VaultStore", () => {
  test("init_loads_status_and_credentials_only_when_unlocked", async () => {
    const creds = [credential({ id: "c1", name: "deploy key" })];

    const a = mockApi([], {
      vaultStatus: async () => locked(),
      listCredentials: async () => creds,
    });
    const lockedStore = new VaultStore(a.api);
    await lockedStore.init();
    expect(lockedStore.status).toEqual(locked());
    expect(lockedStore.credentials).toEqual([]);
    expect(a.calls.listCredentials).toHaveLength(0);

    const b = mockApi([], {
      vaultStatus: async () => unlocked(),
      listCredentials: async () => creds,
    });
    const unlockedStore = new VaultStore(b.api);
    await unlockedStore.init();
    expect(unlockedStore.status?.state).toBe("unlocked");
    expect(unlockedStore.credentials).toEqual(creds);
    expect(b.calls.listCredentials).toHaveLength(1);
  });

  test("unlock_error_keeps_status_locked_and_returns_the_error", async () => {
    const err = apiError("wrong_password", "wrong password");
    const { api, calls } = mockApi([], {
      vaultStatus: async () => locked(),
      vaultUnlock: async () => {
        throw err;
      },
    });
    const store = new VaultStore(api);
    await store.init();
    const result = await store.unlock("nope", true);
    expect(result).toEqual(err);
    expect(calls.vaultUnlock).toEqual([["nope", true]]);
    expect(store.status?.state).toBe("locked");
    expect(store.credentials).toEqual([]);
  });

  test("lock_clears_the_credential_list", async () => {
    const { api } = mockApi([], {
      vaultStatus: async () => unlocked(),
      listCredentials: async () => [credential({ id: "c1" })],
      vaultLock: async () => locked(),
    });
    const store = new VaultStore(api);
    await store.init();
    expect(store.credentials).toHaveLength(1);
    expect(await store.lock()).toBeNull();
    expect(store.status?.state).toBe("locked");
    expect(store.credentials).toEqual([]);
  });

  test("unlock_and_create_load_the_credentials_and_adopt_the_status", async () => {
    const { api, calls } = mockApi([], {
      vaultStatus: async () => vaultStatus(),
      vaultUnlock: async () => unlocked(),
      vaultCreate: async () => unlocked(),
      listCredentials: async () => [credential({ id: "c1" })],
    });
    const store = new VaultStore(api);
    await store.init();
    expect(await store.create("a long master password", false)).toBeNull();
    expect(calls.vaultCreate).toEqual([["a long master password", false]]);
    expect(store.status?.state).toBe("unlocked");
    expect(store.credentials).toHaveLength(1);
  });

  test("change_password_and_auto_unlock_adopt_the_returned_status", async () => {
    const { api, calls } = mockApi([], {
      vaultStatus: async () => unlocked(),
      vaultChangePassword: async () => unlocked(),
      vaultSetAutoUnlock: async () => ({ ...unlocked(), autoUnlock: true }),
    });
    const store = new VaultStore(api);
    await store.init();
    expect(await store.changePassword("old", "new")).toBeNull();
    expect(calls.vaultChangePassword).toEqual([["old", "new"]]);
    expect(await store.setAutoUnlock(true)).toBeNull();
    expect(store.status?.autoUnlock).toBe(true);
  });

  test("restart_required_goes_to_the_global_error_and_is_also_returned", async () => {
    const err = apiError("restart_required", "restart first");
    const { api } = mockApi([], {
      vaultStatus: async () => unlocked(),
      vaultChangePassword: async () => {
        throw err;
      },
    });
    const shown: string[] = [];
    const store = new VaultStore(api, (m) => shown.push(m));
    await store.init();
    expect(await store.changePassword("a", "b")).toEqual(err);
    expect(shown).toEqual(["restart first"]);
  });

  test("calls_run_one_after_another", async () => {
    const order: string[] = [];
    const { api } = mockApi([], {
      vaultStatus: async () => unlocked(),
      vaultLock: async () => {
        order.push("lock:start");
        await new Promise((r) => setTimeout(r, 5));
        order.push("lock:end");
        return locked();
      },
      vaultUnlock: async () => {
        order.push("unlock");
        return unlocked();
      },
    });
    const store = new VaultStore(api);
    await store.init();
    await Promise.all([store.lock(), store.unlock("x", false)]);
    expect(order).toEqual(["lock:start", "lock:end", "unlock"]);
  });

  test("init_failure_shows_a_notice_and_marks_the_vault_unavailable", async () => {
    const { api } = mockApi([], {
      vaultStatus: async () => {
        throw apiError("internal", "boom");
      },
    });
    const shown: string[] = [];
    const store = new VaultStore(api, (m) => shown.push(m));
    await store.init();
    expect(shown).toEqual(["Vault unavailable: boom"]);
    expect(store.status).toBeNull();
    expect(store.unavailable).toBe(true);
    expect(store.error).toBe("boom");
  });

  test("list_failure_after_unlock_shows_a_notice_and_leaves_the_vault_unlocked", async () => {
    const { api } = mockApi([], {
      vaultStatus: async () => locked(),
      vaultUnlock: async () => unlocked(),
      listCredentials: async () => {
        throw apiError("internal", "list broke");
      },
    });
    const shown: string[] = [];
    const store = new VaultStore(api, (m) => shown.push(m));
    await store.init();
    expect(await store.unlock("pw", false)).toBeNull();
    expect(store.status?.state).toBe("unlocked");
    expect(store.credentials).toEqual([]);
    expect(shown).toEqual(["Could not load the credentials: list broke"]);
  });

  test("list_failure_during_init_keeps_the_status_and_shows_a_notice", async () => {
    const { api } = mockApi([], {
      vaultStatus: async () => unlocked(),
      listCredentials: async () => {
        throw apiError("internal", "list broke");
      },
    });
    const shown: string[] = [];
    const store = new VaultStore(api, (m) => shown.push(m));
    await store.init();
    expect(store.status?.state).toBe("unlocked");
    expect(store.unavailable).toBe(false);
    expect(shown).toEqual(["Could not load the credentials: list broke"]);
  });
});
