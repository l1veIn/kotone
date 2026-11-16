<script lang="ts">
  // 「通用 → 外观」：配色、明暗、界面缩放、界面字体。只影响设置窗口；偏好存在本机前端。
  // 界面参照 RambleDesk AppearanceSettingsPanel.svelte（不含工作区背景图片与代码字体）。
  import { Moon, Palette, RotateCcw, Scaling, Sun, SunMoon, Type } from "@lucide/svelte";
  import { Button } from "$lib/components/ui/button";
  import { Input } from "$lib/components/ui/input";
  import * as Select from "$lib/components/ui/select";
  import { appearancePreferences, resetAppearance, updateAppearance } from "./appearancePreferences";
  import {
    PALETTES, THEME_MODES, UI_FONTS, ZOOM_LEVELS, resolveFontFamily,
    type AppearanceSettings, type FontId, type ThemeMode,
  } from "./appearanceSettings";

  let saveError = $state("");
  let restored = $state(false);
  let customFont = $state($appearancePreferences.uiCustomFont);
  const fontFamily = $derived(resolveFontFamily($appearancePreferences));
  const modeIcons = { dark: Moon, light: Sun, system: SunMoon } as const;

  function save(patch: Partial<AppearanceSettings>) {
    saveError = "";
    restored = false;
    try { updateAppearance(patch); }
    catch (cause) { saveError = cause instanceof Error ? cause.message : String(cause); }
  }
  function reset() {
    saveError = "";
    try { resetAppearance(); customFont = ""; restored = true; }
    catch (cause) { saveError = cause instanceof Error ? cause.message : String(cause); }
  }
</script>

<div class="ui-scope space-y-5 text-foreground" data-testid="appearance-settings">
  {#if saveError}
    <p role="alert" class="rounded-lg border border-destructive/40 bg-destructive/10 px-3 py-2 text-xs text-destructive">
      外观设置没能保存（本次仍然生效，重启后恢复默认）：{saveError}
    </p>
  {/if}

  <div class="space-y-2">
    <p class="flex items-center gap-1.5 text-xs font-medium text-muted-foreground"><SunMoon class="size-3.5" />明暗</p>
    <div class="grid grid-cols-3 gap-2" role="group" aria-label="明暗">
      {#each THEME_MODES as mode (mode.id)}
        {@const Icon = modeIcons[mode.id]}
        {@const active = $appearancePreferences.mode === mode.id}
        <button type="button" data-testid={`appearance-mode-${mode.id}`} aria-pressed={active}
          class={["flex items-center justify-center gap-1.5 rounded-lg border px-3 py-2 text-xs transition-colors hover:bg-accent hover:text-accent-foreground focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring",
            active ? "border-primary bg-primary/10 text-primary" : "border-border text-foreground/80"]}
          onclick={() => save({ mode: mode.id as ThemeMode })}>
          <Icon class="size-3.5" />{mode.label}
        </button>
      {/each}
    </div>
  </div>

  <div class="space-y-2">
    <p class="flex items-center gap-1.5 text-xs font-medium text-muted-foreground"><Palette class="size-3.5" />配色</p>
    <div class="grid grid-cols-3 gap-2 sm:grid-cols-4 lg:grid-cols-5" role="group" aria-label="配色">
      {#each PALETTES as palette (palette.id)}
        {@const active = $appearancePreferences.palette === palette.id}
        <button type="button" data-testid={`appearance-palette-${palette.id}`} aria-pressed={active}
          class={["flex min-w-0 items-center gap-2 rounded-lg border px-3 py-2 text-left text-xs transition-colors hover:bg-accent hover:text-accent-foreground focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring",
            active ? "border-primary ring-2 ring-primary/25" : "border-border"]}
          onclick={() => save({ palette: palette.id })}>
          <span class="size-4 shrink-0 rounded-full border border-foreground/15" style:background-color={palette.swatch}></span>
          <span class="truncate">{palette.label}</span>
        </button>
      {/each}
    </div>
    <p class="text-[11px] leading-5 text-muted-foreground">只影响设置窗口；游戏里的悬浮窗与弹幕保持原样。默认是 Kotone 深色。</p>
  </div>

  <div class="grid gap-4 sm:grid-cols-2">
    <div class="space-y-2">
      <p class="flex items-center gap-1.5 text-xs font-medium text-muted-foreground"><Scaling class="size-3.5" />界面缩放</p>
      <Select.Root type="single" value={String($appearancePreferences.zoom)} onValueChange={(value) => save({ zoom: Number(value) })}>
        <Select.Trigger class="w-full" aria-label="界面缩放" data-testid="appearance-zoom">{$appearancePreferences.zoom}%{$appearancePreferences.zoom === 100 ? "（默认）" : ""}</Select.Trigger>
        <Select.Content>
          {#each ZOOM_LEVELS as zoom (zoom)}
            <Select.Item value={String(zoom)} label={`${zoom}%${zoom === 100 ? "（默认）" : ""}`} />
          {/each}
        </Select.Content>
      </Select.Root>
    </div>
    <div class="space-y-2">
      <p class="flex items-center gap-1.5 text-xs font-medium text-muted-foreground"><Type class="size-3.5" />界面字体</p>
      <Select.Root type="single" value={$appearancePreferences.uiFont} onValueChange={(value) => save({ uiFont: value as FontId })}>
        <Select.Trigger class="w-full" aria-label="界面字体" data-testid="appearance-font">{UI_FONTS.find((font) => font.id === $appearancePreferences.uiFont)?.label}</Select.Trigger>
        <Select.Content>
          {#each UI_FONTS as font (font.id)}
            <Select.Item value={font.id} label={font.label} />
          {/each}
        </Select.Content>
      </Select.Root>
    </div>
  </div>

  {#if $appearancePreferences.uiFont === "custom"}
    <div class="space-y-1.5">
      <label for="appearance-custom-font" class="text-xs font-medium text-muted-foreground">自定义字体名称</label>
      <Input id="appearance-custom-font" bind:value={customFont} placeholder="已安装的字体，例如 Microsoft YaHei" maxlength={160}
        onchange={() => save({ uiCustomFont: customFont })} />
      <p class="text-[11px] text-muted-foreground">使用本机已安装的字体；写错或没装时用默认字体。</p>
    </div>
  {/if}

  <p class="rounded-lg border bg-card/60 px-3 py-2 text-sm" style:font-family={fontFamily} data-testid="appearance-preview">
    琴音在这里 · Kotone 0123456789
  </p>

  <div class="flex flex-wrap items-center gap-3">
    <Button variant="outline" size="sm" data-testid="appearance-reset" onclick={reset}><RotateCcw class="size-3.5" />恢复默认外观</Button>
    {#if restored}<span role="status" class="text-xs text-muted-foreground">已恢复默认外观。</span>{/if}
  </div>
</div>
