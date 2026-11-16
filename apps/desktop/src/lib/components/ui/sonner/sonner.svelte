<script lang="ts">
  // shadcn-svelte 的 Sonner 提示（结构参照 RambleDesk lib/components/ui/sonner）。Kotone：每种提示配一张
  // 琴音贴纸（成功=得意 / 信息=指引 / 警告=思考 / 错误=惊讶），颜色跟随设置窗口的外观变量。
  import { Toaster } from "svelte-sonner";
  import stickerProud from "../../../../assets/brand/stickers/proud.webp";
  import stickerPointing from "../../../../assets/brand/stickers/pointing.webp";
  import stickerThinking from "../../../../assets/brand/stickers/thinking.webp";
  import stickerAmazed from "../../../../assets/brand/stickers/amazed.webp";

  let { theme = "dark" }: { theme?: "dark" | "light" } = $props();
</script>

{#snippet sticker(source: string)}
  <img src={source} alt="" class="kotone-toast-sticker" aria-hidden="true" />
{/snippet}
{#snippet successIcon()}{@render sticker(stickerProud)}{/snippet}
{#snippet infoIcon()}{@render sticker(stickerPointing)}{/snippet}
{#snippet warningIcon()}{@render sticker(stickerThinking)}{/snippet}
{#snippet errorIcon()}{@render sticker(stickerAmazed)}{/snippet}

<Toaster
  position="bottom-right"
  dir="ltr"
  offset={{ bottom: 20, right: 20 }}
  {theme}
  closeButton
  {successIcon}
  {infoIcon}
  {warningIcon}
  {errorIcon}
  toastOptions={{ duration: 4_000, closeButtonAriaLabel: "关闭提示" }}
/>

<style>
  :global([data-sonner-toaster]) {
    z-index: 140 !important;
  }
  :global([data-sonner-toast][data-styled="true"]) {
    border: 1px solid var(--color-hairline) !important;
    color: var(--color-foreground) !important;
    background: var(--color-popover) !important;
    border-radius: 16px !important;
    box-shadow: 0 16px 40px -12px rgb(0 0 0 / 0.45) !important;
    font-size: 13px !important;
    line-height: 1.6 !important;
    gap: 10px !important;
    padding: 10px 14px !important;
  }
  :global([data-sonner-toast][data-styled="true"] [data-icon]) {
    width: 44px !important;
    height: 44px !important;
    margin: -4px 2px -4px -4px !important;
  }
  :global(.kotone-toast-sticker) {
    width: 44px;
    height: 44px;
    max-width: none;
    object-fit: contain;
  }
  :global([data-sonner-toast][data-type="success"]) {
    border-color: color-mix(in oklab, var(--color-primary) 45%, transparent) !important;
  }
  :global([data-sonner-toast][data-type="success"] [data-title]) {
    color: var(--color-primary) !important;
  }
  :global([data-sonner-toast][data-type="warning"]) {
    border-color: color-mix(in oklab, var(--color-warning) 50%, transparent) !important;
  }
  :global([data-sonner-toast][data-type="warning"] [data-title]) {
    color: var(--color-warning) !important;
  }
  :global([data-sonner-toast][data-type="error"]) {
    border-color: color-mix(in oklab, var(--color-brand-pink) 55%, transparent) !important;
  }
  :global([data-sonner-toast][data-type="error"] [data-title]) {
    color: var(--color-brand-pink) !important;
  }
  :global([data-sonner-toast] [data-close-button]) {
    background: var(--color-popover) !important;
    border-color: var(--color-hairline) !important;
    color: var(--color-muted-foreground) !important;
  }
</style>
