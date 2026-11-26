<script lang="ts" module>
  /** 问要不要下载本地文字识别模型的那张卡片（推送 `augment-pick`，后端 plugins/lol/src/pushes/augment_pick.rs 的 `ocr_card`）。 */
  export interface AugmentOcrData {
    state: "ask" | "downloading" | "ready" | "declined" | "failed";
    model?: string;
    sizeMb?: number;
    reason?: string;
  }
  export function isAugmentOcr(value: unknown): value is AugmentOcrData {
    return !!value && typeof value === "object" && typeof (value as AugmentOcrData).state === "string";
  }
</script>

<script lang="ts">
  // 文字识别模型（`ui://kotone/lol/augment-ocr`，ADR 043 D7）：海斗三选一固定用本地文字识别（和全局的识别设置无关）；
  // 一个本地模型都没装时，插件推这张卡片问一句“要下载吗”。下载 / 不用都是只给页面的工具 `lol.augment_ocr`：
  // 下载在后台，走本体的模型下载，不改「设置 → 模型 → 文字识别」里的选择；不用记进插件设置，以后不再问。
  // 卡片的后续（正在下载、下好了、没下成）插件会原地更新推过来；点按钮时工具也马上回一份。
  import { onMount } from "svelte";
  import type { DisplayMode, Session, ToolResult } from "../host";
  import { m } from "../i18n.svelte";

  /** 对外（MCP）的工具名（Kotone 里宿主的桥还原成 `lol.augment_ocr`）。 */
  const TOOL_AUGMENT_OCR = "kotone_lol_augment_ocr";

  let {
    session,
    initial,
    display,
  }: {
    session: Session;
    initial: Promise<ToolResult | null> | null;
    display: DisplayMode;
  } = $props();

  let card = $state<AugmentOcrData | null>(null);
  let busy = $state(false);
  let error = $state("");
  const full = $derived(display !== "inline");

  function show(result: ToolResult | null) {
    if (result && isAugmentOcr(result.structuredContent)) card = result.structuredContent;
  }

  async function call(action: "status" | "download" | "decline") {
    if (busy) return;
    busy = true;
    error = "";
    try {
      show(await (action === "status" ? session.firstCall : session.call)(TOOL_AUGMENT_OCR, { action }));
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      busy = false;
    }
  }

  onMount(() => {
    session.onResult(show);
    if (initial) void initial.then((result) => (result ? show(result) : call("status")));
    else void call("status");
  });

  const size = $derived(String(card?.sizeMb ?? ""));
</script>

<div class="app" data-testid="lol-app-augment-ocr" data-state={card?.state ?? ""}>
  <div class="ocr" class:full>
    {#if card}
      {#if card.state === "ask" || card.state === "failed"}
        <p class="q">{card.state === "ask" ? m.plugin_lol_augment_ocr_ask({ size }) : m.plugin_lol_augment_ocr_failed({ reason: card.reason ?? "" })}</p>
        <p class="why">{m.plugin_lol_augment_ocr_why()}</p>
        <div class="actions">
          <button type="button" class="btn primary" disabled={busy} data-testid="lol-augment-ocr-download" onclick={() => void call("download")}>
            {card.state === "ask" ? m.plugin_lol_augment_ocr_download() : m.plugin_lol_retry()}
          </button>
          <button type="button" class="btn" disabled={busy} data-testid="lol-augment-ocr-decline" onclick={() => void call("decline")}>
            {m.plugin_lol_augment_ocr_decline()}
          </button>
        </div>
      {:else if card.state === "downloading"}
        <p class="q" aria-busy="true">{m.plugin_lol_augment_ocr_downloading({ model: card.model ?? "", size })}</p>
      {:else if card.state === "ready"}
        <p class="q good">{m.plugin_lol_augment_ocr_ready()}</p>
      {:else}
        <p class="q muted">{m.plugin_lol_augment_ocr_declined()}</p>
      {/if}
    {/if}
    {#if error}<p class="card-line danger" role="alert">{error}</p>{/if}
  </div>
</div>

<style>
  .ocr {
    display: grid;
    gap: 6px;
    padding: 10px 12px 12px;
  }
  .ocr.full {
    max-width: 560px;
    padding: var(--lol-sp-6);
  }
  .q {
    margin: 0;
    font-size: var(--lol-fs-md);
    line-height: var(--lol-lh-md);
    font-weight: var(--lol-fw-medium);
  }
  .q.muted {
    font-weight: var(--lol-fw-normal);
    color: var(--lol-text-2);
  }
  .why {
    margin: 0;
    font-size: var(--lol-fs-xs);
    line-height: var(--lol-lh-xs);
    color: var(--lol-text-3);
  }
  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    margin-top: 4px;
  }
</style>
