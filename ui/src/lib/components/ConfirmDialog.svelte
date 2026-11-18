<script lang="ts">
  let {
    title,
    detail,
    confirmLabel,
    onConfirm,
    onCancel,
  }: {
    title: string;
    detail?: string;
    confirmLabel: string;
    onConfirm: () => void;
    onCancel: () => void;
  } = $props();

  const uid = $props.id();

  let dialog = $state<HTMLElement>();
  let cancelButton = $state<HTMLButtonElement>();
  let confirmButton = $state<HTMLButtonElement>();

  $effect(() => {
    const root = dialog;
    if (!root) return;
    const opener = document.activeElement as HTMLElement | null;
    // the safe choice has the initial focus
    cancelButton?.focus();
    return () => {
      const active = document.activeElement;
      if (!active || active === document.body || root.contains(active)) opener?.focus?.();
    };
  });

  function onkeydown(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      onCancel();
    } else if (e.key === "Tab") {
      // keep focus inside the dialog
      e.preventDefault();
      const buttons = [cancelButton, confirmButton];
      const index = buttons.indexOf(document.activeElement as HTMLButtonElement);
      const next = e.shiftKey ? index - 1 + buttons.length : index + 1;
      buttons[next % buttons.length]?.focus();
    }
  }
</script>

<div class="overlay">
  <div
    bind:this={dialog}
    class="dialog"
    role="alertdialog"
    aria-modal="true"
    aria-labelledby="{uid}-title"
    aria-describedby={detail ? `${uid}-detail` : undefined}
    tabindex="-1"
    {onkeydown}
  >
    <h2 class="title" id="{uid}-title">{title}</h2>
    {#if detail}<p class="detail" id="{uid}-detail">{detail}</p>{/if}
    <div class="actions">
      <button bind:this={cancelButton} type="button" class="btn" onclick={onCancel}>Cancel</button>
      <button bind:this={confirmButton} type="button" class="btn danger" onclick={onConfirm}>
        {confirmLabel}
      </button>
    </div>
  </div>
</div>

<style>
  .overlay {
    position: fixed;
    inset: 0;
    z-index: 30;
    display: flex;
    align-items: center;
    justify-content: center;
    background: var(--color-overlay);
  }

  .dialog {
    width: 360px;
    max-width: calc(100vw - var(--space-6));
    padding: var(--space-5);
    background: var(--color-surface-raised);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-md);
    box-shadow: var(--shadow-raised);
    outline: none;
  }

  .title {
    margin: 0;
    font-size: var(--text-md);
    font-weight: var(--weight-strong);
    overflow-wrap: anywhere;
  }

  .detail {
    margin: var(--space-2) 0 0;
    color: var(--color-text-muted);
    font-size: var(--text-sm);
  }

  .actions {
    display: flex;
    justify-content: flex-end;
    gap: var(--space-3);
    margin-top: var(--space-5);
  }

  .danger {
    color: var(--color-danger);
  }
</style>
