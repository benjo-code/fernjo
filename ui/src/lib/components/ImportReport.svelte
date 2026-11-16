<script lang="ts">
  import type { Report, ReportLine } from "../types";

  let { report }: { report: Report } = $props();

  const uid = $props.id();

  const c = $derived(report.counts);
  const connections = $derived(c.ssh + c.rdp);

  const plural = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;
  const summary = $derived(
    `${plural(c.folders, "folder", "folders")}, ${plural(connections, "connection", "connections")} (${c.ssh} SSH, ${c.rdp} RDP), ${plural(c.credentials, "credential", "credentials")}${c.sharedCredentials > 0 ? ` (${c.sharedCredentials} shared)` : ""}`,
  );

  // collapsed until opened; the lines are Rust's text, shown as they are
  let open = $state<Record<string, boolean>>({});

  const lists = $derived<{ key: string; title: string; lines: ReportLine[] }[]>([
    { key: "skipped", title: "Skipped", lines: report.skipped },
    { key: "partial", title: "Partly mapped", lines: report.partial },
    { key: "problems", title: "Problems", lines: report.problems },
  ]);
</script>

<div class="report">
  <p class="counts">{summary}</p>
  {#each lists as list (list.key)}
    {#if list.lines.length > 0}
      <div class="list">
        <button
          type="button"
          class="toggle"
          aria-expanded={open[list.key] === true}
          aria-controls="{uid}-{list.key}"
          onclick={() => (open[list.key] = !open[list.key])}
        >
          <span class="chev" aria-hidden="true">{open[list.key] ? "▾" : "▸"}</span>
          {list.title} ({list.lines.length})
        </button>
        {#if open[list.key]}
          <ul id="{uid}-{list.key}">
            {#each list.lines as line, i (i)}
              <li><span class="path">{line.path}</span> <span>{line.text}</span></li>
            {/each}
          </ul>
        {/if}
      </div>
    {/if}
  {/each}
</div>

<style>
  .report {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .counts {
    margin: 0;
    font-size: var(--text-sm);
  }

  .toggle {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: 0;
    border: 0;
    background: transparent;
    color: var(--color-text);
    font: inherit;
    font-size: var(--text-sm);
    font-weight: var(--weight-medium);
    cursor: default;
  }

  .toggle:hover {
    color: var(--color-text-muted);
  }

  .chev {
    width: 10px;
  }

  ul {
    max-height: 160px;
    margin: var(--space-2) 0 0;
    padding: var(--space-2) var(--space-3);
    overflow-y: auto;
    list-style: none;
    background: var(--color-surface);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-sm);
    color: var(--color-text-muted);
    font-size: var(--text-sm);
    overflow-wrap: anywhere;
  }

  li + li {
    margin-top: var(--space-2);
  }

  .path {
    color: var(--color-text);
  }
</style>
