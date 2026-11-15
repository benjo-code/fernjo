<script lang="ts">
  import type { TreeStore } from "../store.svelte";
  import type { VaultStore } from "../vault.svelte";
  import type { FieldInfo, NodePatch, Protocol } from "../types";
  import Icon from "./Icon.svelte";
  import PropertyRow from "./PropertyRow.svelte";
  import TextField from "./TextField.svelte";

  let { store, vault }: { store: TreeStore; vault: VaultStore } = $props();

  let open = $state(true);
  let errors = $state<Record<string, string | null>>({});

  const detail = $derived(store.detail);
  const nodeId = $derived(detail?.id ?? null);
  const kindLabel = $derived(
    !detail
      ? ""
      : detail.kind === "folder"
        ? "Folder"
        : `${detail.protocol === "rdp" ? "RDP" : "SSH"} connection`,
  );
  const isFolder = $derived(detail?.kind === "folder");
  const showKeepalive = $derived(isFolder || detail?.protocol === "ssh");

  // an error belongs to the node it was raised on
  $effect(() => {
    void nodeId;
    errors = {};
  });

  const setError = (field: string, message: string | null) => {
    errors = { ...errors, [field]: message };
  };

  /** Sends one patch; a rejection shows inline under the field instead of in the global bar. */
  async function save(field: string, patch: NodePatch): Promise<boolean> {
    if (!detail) return false;
    setError(field, null);
    const err = await store.update(detail.id, patch, { silent: true });
    if (err) setError(field, err.message);
    return err === null;
  }

  async function commitText(
    field: string,
    key: "username",
    text: string,
    info: FieldInfo<string>,
  ): Promise<boolean> {
    const value = text.trim();
    if (value === "") return info.local === null ? true : save(field, { [key]: null });
    if (value === info.local) return true;
    return save(field, { [key]: value });
  }

  async function commitNumber(
    field: string,
    key: "port" | "keepaliveSecs",
    label: string,
    text: string,
    info: FieldInfo<number>,
  ): Promise<boolean> {
    const value = text.trim();
    if (value === "") return info.local === null ? true : save(field, { [key]: null });
    if (!/^\d+$/.test(value)) {
      setError(field, `${label} must be a whole number`);
      return false;
    }
    const n = Number(value);
    if (n > 0xffffffff) {
      setError(field, `${label} is too large`);
      return false;
    }
    if (n === info.local) return true;
    return save(field, { [key]: n });
  }

  // The credential row: Rust reports the effective id and its source; the name comes from the
  // unlocked list. Only a name is ever shown, never the id or a secret.
  const vaultState = $derived(vault.status?.state ?? null);
  const unlocked = $derived(vaultState === "unlocked");
  const credentialId = $derived(detail?.credential.effective ?? null);
  const credentialName = $derived(
    credentialId === null
      ? null
      : (vault.credentials.find((c) => c.id === credentialId)?.name ?? null),
  );

  async function chooseCredential(select: HTMLSelectElement) {
    if (!detail) return;
    const chosen = select.value;
    const ok =
      chosen === ""
        ? // "None" clears a local value (same as Inherit); with nothing local there is nothing to clear
          detail.credential.local === null || (await save("credential", { credential: null }))
        : chosen === detail.credential.local || (await save("credential", { credential: chosen }));
    if (!ok || chosen === "" || chosen === detail.credential.local) {
      select.value = detail.credential.effective ?? "";
    }
  }

  const show = (n: number | null) => (n === null ? "" : String(n));
</script>

<section class="panel" aria-label="Properties">
  <button type="button" class="header" aria-expanded={open} onclick={() => (open = !open)}>
    <span class="chevron" class:closed={!open}><Icon name="folder-chevron" size={14} /></span>
    <span class="title" title={detail?.name}>{detail?.name ?? "Properties"}</span>
    <span class="kind">{kindLabel}</span>
  </button>

  {#if open}
    <div class="body">
      {#if !detail}
        <p class="empty">Select a connection or folder to see its settings.</p>
      {:else}
        <PropertyRow name="name" label="Name" error={errors.name}>
          {#snippet children(id)}
            <TextField
              {id}
              label="Name"
              value={detail.name}
              onCommit={(text) =>
                text === detail.name ? Promise.resolve(true) : save("name", { name: text })}
              onedit={() => setError("name", null)}
            />
          {/snippet}
        </PropertyRow>

        <PropertyRow name="description" label="Description" error={errors.description}>
          {#snippet children(id)}
            <TextField
              {id}
              label="Description"
              value={detail.description}
              onCommit={(text) =>
                text === detail.description
                  ? Promise.resolve(true)
                  : save("description", { description: text })}
              onedit={() => setError("description", null)}
            />
          {/snippet}
        </PropertyRow>

        {#if !isFolder}
          <PropertyRow name="protocol" label="Protocol" error={errors.protocol}>
            {#snippet children(id)}
              <select
                {id}
                aria-label="Protocol"
                value={detail.protocol}
                onchange={async (e) => {
                  const select = e.currentTarget;
                  const ok = await save("protocol", { protocol: select.value as Protocol });
                  if (!ok) select.value = detail.protocol ?? "ssh";
                }}
              >
                <option value="ssh">SSH</option>
                <option value="rdp">RDP</option>
              </select>
            {/snippet}
          </PropertyRow>

          <PropertyRow name="host" label="Host" error={errors.host}>
            {#snippet children(id)}
              <TextField
                {id}
                label="Host"
                value={detail.host ?? ""}
                onCommit={(text) =>
                  text === detail.host ? Promise.resolve(true) : save("host", { host: text })}
                onedit={() => setError("host", null)}
              />
            {/snippet}
          </PropertyRow>
        {/if}

        <PropertyRow
          name="port"
          label="Port"
          source={detail.port.source}
          hasValue={detail.port.effective !== null}
          error={errors.port}
          onreset={() => void save("port", { port: null })}
        >
          {#snippet children(id)}
            <TextField
              {id}
              label="Port"
              inputmode="numeric"
              placeholder={isFolder ? "22 (SSH), 3389 (RDP)" : ""}
              value={show(detail.port.effective)}
              onCommit={(text) => commitNumber("port", "port", "Port", text, detail.port)}
              onedit={() => setError("port", null)}
            />
          {/snippet}
        </PropertyRow>

        <PropertyRow
          name="username"
          label="Username"
          source={detail.username.source}
          hasValue={detail.username.effective !== null}
          error={errors.username}
          onreset={() => void save("username", { username: null })}
        >
          {#snippet children(id)}
            <TextField
              {id}
              label="Username"
              value={detail.username.effective ?? ""}
              onCommit={(text) => commitText("username", "username", text, detail.username)}
              onedit={() => setError("username", null)}
            />
          {/snippet}
        </PropertyRow>

        {#if showKeepalive}
          <PropertyRow
            name="keepalive"
            label="Keepalive"
            source={detail.keepaliveSecs.source}
            hasValue={detail.keepaliveSecs.effective !== null}
            error={errors.keepalive}
            onreset={() => void save("keepalive", { keepaliveSecs: null })}
          >
            {#snippet children(id)}
              <TextField
                {id}
                label="Keepalive"
                title="Seconds between keepalive packets; 0 turns keepalive off"
                inputmode="numeric"
                value={show(detail.keepaliveSecs.effective)}
                onCommit={(text) =>
                  commitNumber(
                    "keepalive",
                    "keepaliveSecs",
                    "Keepalive",
                    text,
                    detail.keepaliveSecs,
                  )}
                onedit={() => setError("keepalive", null)}
              />
            {/snippet}
          </PropertyRow>
        {/if}

        <PropertyRow
          name="credential"
          label="Credential"
          source={detail.credential.source}
          hasValue={detail.credential.effective !== null}
          error={errors.credential}
          onreset={unlocked ? () => void save("credential", { credential: null }) : undefined}
        >
          {#snippet children(id)}
            {#if !unlocked}
              <span {id} class="static"
                >{vaultState === "locked" ? "Vault locked" : "No vault"}</span
              >
            {:else if credentialId !== null && credentialName === null}
              <span {id} class="static">Missing credential</span>
            {:else}
              <select
                {id}
                aria-label="Credential"
                value={credentialId ?? ""}
                onchange={(e) => void chooseCredential(e.currentTarget)}
              >
                <option value="">None</option>
                {#each vault.credentials as c (c.id)}
                  <option value={c.id}>{c.name}</option>
                {/each}
              </select>
            {/if}
          {/snippet}
        </PropertyRow>
      {/if}
    </div>
  {/if}
</section>

<style>
  .panel {
    display: flex;
    flex: 0 1 auto;
    flex-direction: column;
    min-height: 26px;
    border-top: 1px solid var(--color-border);
  }

  .header {
    display: flex;
    flex: none;
    align-items: center;
    gap: var(--space-2);
    width: 100%;
    height: 26px;
    padding: 0 var(--space-3);
    border: 0;
    background: transparent;
    color: var(--color-text);
    font: inherit;
    text-align: left;
    cursor: default;
  }

  .chevron {
    display: flex;
    color: var(--color-text-muted);
  }

  .chevron.closed {
    transform: rotate(-90deg);
  }

  .title {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    font-weight: var(--weight-strong);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .kind {
    flex: none;
    color: var(--color-text-muted);
    font-size: var(--text-sm);
  }

  .body {
    flex: 1 1 auto;
    min-height: 0;
    padding: var(--space-2) var(--space-3) var(--space-3) 10px;
    overflow-y: auto;
  }

  .empty {
    margin: var(--space-2) 0;
    color: var(--color-text-muted);
    font-size: var(--text-sm);
  }

  .static {
    display: block;
    overflow: hidden;
    color: var(--color-text-muted);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  select {
    width: 100%;
    height: 22px;
    padding: 0 var(--space-2);
    border: 1px solid var(--color-border-input);
    border-radius: var(--radius-sm);
    background: var(--color-surface);
    color: var(--color-text);
    font: inherit;
  }
</style>
