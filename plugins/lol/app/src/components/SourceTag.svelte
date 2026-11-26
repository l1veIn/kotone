<script lang="ts" module>
  import { m } from "../i18n.svelte";
  import type { SourceId } from "../types";

  /** 来源的名字（messages 里的 `plugin_lol_source_tag_*`；一个个写出来，terminology.spec 认得出用到了哪些）。 */
  export function sourceName(id: SourceId): string {
    switch (id) {
      case "opgg":
        return m.plugin_lol_source_tag_opgg();
      case "aramhex":
        return m.plugin_lol_source_tag_aramhex();
      case "qq101":
        return m.plugin_lol_source_tag_qq101();
      default:
        return m.plugin_lol_source_tag_client();
    }
  }
  /** 来源后面的一个小注（国服 / 全球 / 许可证 / 本机，只读）。 */
  export function sourceNote(id: SourceId): string {
    switch (id) {
      case "opgg":
        return m.plugin_lol_source_note_opgg();
      case "aramhex":
        return m.plugin_lol_source_note_aramhex();
      case "qq101":
        return m.plugin_lol_source_note_qq101();
      default:
        return m.plugin_lol_source_note_client();
    }
  }
</script>

<script lang="ts">
  // 来源标签（ADR 043 D8；docs/design/lol-app.md 第一节 11）：每块数据一个小标签写它从哪来，许可证要求的署名就在这里
  // （OP.GG：标志 + op.gg 的链接，opgg-mcp#26 里答应过；aramhex-data：CC BY-NC 4.0 的链接）。能换来源的块，标签本身
  // 就是开关：点开一个小菜单选另一个来源，页面记住（`lol.source` → 插件设置）。同一块一次只用一个来源。
  // 风险说明不在这里（插件的“风险和许可”弹窗）。
  import { ARAMHEX_URL, OPGG_URL, type SourceChoice } from "../types";

  let {
    id,
    choice = null,
    date = "",
    stale = false,
    busy = false,
    onPick,
    onLink,
  }: {
    id: SourceId;
    /** 能换来源时给（选项里不止一个才显示菜单）。 */
    choice?: SourceChoice | null;
    /** 数据的日期（“20261124”）或版本。 */
    date?: string;
    stale?: boolean;
    busy?: boolean;
    onPick?: (id: SourceId) => void;
    onLink?: (url: string) => void;
  } = $props();

  let open = $state(false);
  let root = $state<HTMLElement | null>(null);
  const switchable = $derived(!!choice && !!onPick && choice.options.length > 1);
  const link = $derived(
    id === "opgg" ? { url: OPGG_URL, text: "op.gg" } : id === "aramhex" ? { url: ARAMHEX_URL, text: m.plugin_lol_source_note_aramhex() } : null,
  );
  const note = $derived(id === "aramhex" ? "" : sourceNote(id));
  const menuTitle = $derived(choice?.kind === "mayhemAugments" ? m.plugin_lol_source_menu_mayhem_augments() : m.plugin_lol_source_menu_rift());

  function follow(event: MouseEvent, url: string) {
    if (!onLink) return;
    event.preventDefault();
    onLink(url);
  }
  function pick(next: SourceId) {
    open = false;
    if (next !== id) onPick?.(next);
  }
  // 菜单开着时：点别处、按 Esc 关掉
  $effect(() => {
    if (!open) return;
    const away = (event: MouseEvent) => {
      if (root && !root.contains(event.target as Node)) open = false;
    };
    const key = (event: KeyboardEvent) => {
      if (event.key === "Escape") open = false;
    };
    window.addEventListener("pointerdown", away, true);
    window.addEventListener("keydown", key);
    return () => {
      window.removeEventListener("pointerdown", away, true);
      window.removeEventListener("keydown", key);
    };
  });
</script>

{#snippet mark(source: SourceId)}
  {#if source === "opgg"}
    <!-- OP.GG 的字标（它自己的名字，按它的蓝底白字画；不是我们的装饰） -->
    <span class="st-opgg" aria-hidden="true">OP.GG</span>
  {:else}
    <span class="st-name">{sourceName(source)}</span>
  {/if}
{/snippet}

<span class="st-src" bind:this={root} data-testid="lol-source-tag" data-source={id} data-kind={choice?.kind ?? ""}>
  {#if switchable}
    <button
      type="button"
      class="st-tag st-switch"
      aria-haspopup="menu"
      aria-expanded={open}
      title={m.plugin_lol_source_switch()}
      disabled={busy}
      data-testid="lol-source-switch"
      onclick={() => (open = !open)}
    >
      {@render mark(id)}{#if id === "opgg"}<span class="sr-only">{sourceName(id)}</span>{/if}
      {#if note}<span class="st-note">{note}</span>{/if}
      <svg class="st-chev" viewBox="0 0 12 12" aria-hidden="true"><path d="M3 4.5 6 7.5 9 4.5" /></svg>
    </button>
  {:else}
    <span class="st-tag" title={m.plugin_lol_source_title({ name: sourceName(id) })}>
      {@render mark(id)}{#if id === "opgg"}<span class="sr-only">{sourceName(id)}</span>{/if}
      {#if note}<span class="st-note">{note}</span>{/if}
    </span>
  {/if}
  {#if date}<span class="st-date num">{date.slice(0, 10)}</span>{/if}
  {#if stale}<span class="st-date">{m.plugin_lol_stale()}</span>{/if}
  {#if link}
    <a class="st-link" href={link.url} target="_blank" rel="noopener noreferrer" title={m.plugin_lol_source_link({ site: link.text })} data-testid="lol-source-link" onclick={(e) => follow(e, link.url)}>{link.text}</a>
  {/if}
  {#if open && choice}
    <span class="st-menu" role="menu" aria-label={menuTitle} data-testid="lol-source-menu">
      <span class="st-menu-title">{menuTitle}</span>
      {#each choice.options as o (o.id)}
        <button
          type="button"
          role="menuitemradio"
          aria-checked={o.id === id}
          disabled={!o.available}
          class="st-opt"
          data-testid="lol-source-option"
          data-source={o.id}
          onclick={() => pick(o.id)}
        >
          <span class="st-check" aria-hidden="true">{o.id === id ? "✓" : ""}</span>
          {@render mark(o.id)}{#if o.id === "opgg"}<span class="sr-only">{sourceName(o.id)}</span>{/if}
          <span class="st-note">{sourceNote(o.id)}</span>
          {#if !o.available}<span class="st-off">{m.plugin_lol_source_needs_opgg()}</span>{/if}
        </button>
      {/each}
    </span>
  {/if}
</span>

<style>
  .st-src {
    position: relative;
    display: inline-flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 4px 6px;
    font-size: var(--lol-fs-xs);
    line-height: 18px;
    color: var(--lol-text-3);
    white-space: nowrap;
  }
  .st-tag {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    height: 20px;
    padding: 0 6px;
    border: 1px solid var(--lol-line);
    border-radius: var(--lol-r-xs);
    background: var(--lol-bg-2);
    color: var(--lol-text-2);
    font-size: var(--lol-fs-xs);
  }
  .st-tag.st-switch {
    cursor: pointer;
  }
  .st-tag.st-switch:hover:not(:disabled) {
    border-color: var(--lol-line-2);
    color: var(--lol-text);
  }
  .st-tag.st-switch:focus-visible {
    outline: 2px solid var(--lol-ring);
    outline-offset: 1px;
  }
  .st-name {
    color: var(--lol-text);
    font-weight: var(--lol-fw-medium);
  }
  .st-note {
    color: var(--lol-text-3);
  }
  .st-chev {
    width: 10px;
    height: 10px;
    stroke: currentColor;
    fill: none;
    stroke-width: 1.6;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
  /* OP.GG 的字标：蓝底白字粗体（它的品牌色） */
  .st-opgg {
    display: inline-block;
    padding: 0 4px;
    border-radius: 2px;
    background: #5383e8;
    color: #fff;
    font-size: 10px;
    font-weight: 700;
    line-height: 14px;
    letter-spacing: 0.02em;
  }
  .st-date {
    color: var(--lol-text-3);
  }
  .st-link {
    color: inherit;
    text-decoration: underline;
    text-decoration-color: var(--lol-line-2);
    text-underline-offset: 2px;
    cursor: pointer;
  }
  /* 能换来源的标签都在那块标题的右边：菜单往左展开，不出页面 */
  .st-menu {
    position: absolute;
    top: calc(100% + 4px);
    right: 0;
    z-index: 5;
    display: grid;
    gap: 2px;
    min-width: 220px;
    padding: 6px;
    border: 1px solid var(--lol-line-2);
    border-radius: var(--lol-r-md);
    background: var(--lol-bg);
    box-shadow: var(--lol-shadow-pop);
    white-space: normal;
  }
  .st-menu-title {
    padding: 2px 6px 4px;
    color: var(--lol-text-2);
  }
  .st-opt {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
    padding: 4px 6px;
    border: 0;
    border-radius: var(--lol-r-xs);
    background: transparent;
    text-align: left;
    cursor: pointer;
  }
  .st-opt:hover:not(:disabled) {
    background: var(--lol-hover);
  }
  .st-opt:disabled {
    cursor: default;
    opacity: 0.7;
  }
  .st-check {
    width: 10px;
    color: var(--lol-text);
  }
  .st-off {
    flex-basis: 100%;
    padding-left: 16px;
    color: var(--lol-warn);
  }
</style>
