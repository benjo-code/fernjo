<script lang="ts">
  import { untrack } from "svelte";
  import type { SessionStore, Tab } from "../sessions.svelte";
  import type { Prompt } from "../types";
  import PasswordField from "./PasswordField.svelte";
  import PromptFrame from "./PromptFrame.svelte";

  type Asked = Extract<
    Prompt,
    { kind: "keyboardInteractive" | "password" | "keyPassphrase" | "username" }
  >;

  let {
    tab,
    sessions,
    prompt,
  }: {
    tab: Tab;
    sessions: SessionStore;
    prompt: Asked;
  } = $props();

  const uid = $props.id();

  interface Field {
    label: string;
    /** Shown as typed (the server asked for echo, or it is a user name). */
    plain: boolean;
  }

  // What the prompt asks for. Only labels live here, never an answer.
  const view = $derived.by(() => {
    switch (prompt.kind) {
      case "keyboardInteractive":
        return {
          title: "Authentication required",
          lead: prompt.name,
          detail: prompt.instructions,
          fields: prompt.prompts.map<Field>((p) => ({
            label: p.text.trim().replace(/:$/, "").trim() || "Response",
            plain: p.echo,
          })),
        };
      case "password":
        return {
          title: "Password required",
          lead: `${prompt.user}@${prompt.host}`,
          detail: "",
          fields: [{ label: "Password", plain: false }],
        };
      case "keyPassphrase":
        return {
          title: "Key passphrase required",
          lead: "",
          detail: "The private key for this connection is protected by a passphrase.",
          fields: [{ label: "Key passphrase", plain: false }],
        };
      case "username":
        return {
          title: "User name required",
          lead: prompt.host,
          detail: "",
          fields: [{ label: "User name", plain: true }],
        };
    }
  });

  // Typed answers live only in these bound inputs and are emptied before they are sent.
  let values = $state<string[]>(untrack(() => view.fields.map(() => "")));
  let root: HTMLElement | undefined = $state();

  $effect(() => {
    untrack(() => root)
      ?.querySelector("input")
      ?.focus();
  });

  function submit(e: SubmitEvent) {
    e.preventDefault();
    const typed = [...values];
    values = values.map(() => "");
    if (prompt.kind === "username") {
      sessions.answer(tab.id, { kind: "text", value: typed[0] ?? "" });
    } else {
      sessions.answer(tab.id, { kind: "secrets", values: typed });
    }
  }

  function cancel() {
    values = values.map(() => "");
    sessions.answer(tab.id, { kind: "cancel" });
  }

  function onkeydown(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      cancel();
    }
  }
</script>

<PromptFrame bind:root title={view.title} icon="lock" {onkeydown}>
  {#if view.lead}<p class="lead">{view.lead}</p>{/if}
  {#if view.detail}<p class="body">{view.detail}</p>{/if}
  <form onsubmit={submit}>
    {#each view.fields as field, i (i)}
      {#if field.plain}
        <div class="field">
          <label for="{uid}-f{i}">{field.label}</label>
          <input
            id="{uid}-f{i}"
            type="text"
            autocomplete="off"
            spellcheck="false"
            bind:value={values[i]}
          />
        </div>
      {:else}
        <PasswordField id="{uid}-f{i}" label={field.label} bind:value={values[i]!} />
      {/if}
    {/each}
    <div class="actions">
      <button type="submit" class="btn primary">Continue</button>
      <button type="button" class="btn" onclick={cancel}>Cancel</button>
    </div>
  </form>
</PromptFrame>

<style>
  .lead {
    margin: 0;
    overflow-wrap: anywhere;
  }

  .body {
    margin: 0;
    color: var(--color-text-muted);
    font-size: var(--text-sm);
    line-height: 1.3;
    overflow-wrap: anywhere;
    white-space: pre-line;
  }

  form {
    display: flex;
    flex-direction: column;
    gap: 14px;
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  label {
    color: var(--color-text-muted);
    font-size: var(--text-sm);
  }

  input {
    box-sizing: border-box;
    width: 100%;
    height: 28px;
    padding: 0 var(--space-3);
    background: var(--color-surface);
    border: 1px solid var(--color-border-input);
    border-radius: var(--radius-sm);
    color: var(--color-text);
    font: inherit;
  }

  input:focus {
    outline: 1px solid var(--color-focus);
    outline-offset: -1px;
  }

  .actions {
    display: flex;
    gap: var(--space-3);
  }

  .primary {
    background: var(--color-accent);
    border-color: var(--color-accent);
    color: var(--color-accent-text);
  }

  .primary:hover {
    background: var(--color-accent);
  }
</style>
