<script lang="ts">
  // 「通用 → 外观」：配色、明暗、界面缩放、界面字体。只影响设置窗口；偏好存在本机前端。
  // 界面参照 RambleDesk AppearanceSettingsPanel.svelte（不含工作区背景图片与代码字体）。
  import { Moon, Palette, RotateCcw, Scaling, Sun, SunMoon, Type } from "@lucide/svelte";
  import { Button } from "$lib/components/ui/button";
  import { Input } from "$lib/components/ui/input";
  import * as Select from "$lib/components/ui/select";
  import { m } from "$lib/paraglide/messages.js";
  import { Languages } from "@lucide/svelte";
  import { LANGUAGES, changeLocale, currentLocale } from "$lib/locale";
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

<!-- 放在「通用 → 外观」设置卡里：每一项一行，行间细分隔（与其它设置卡一致）。 -->
<div class="ui-scope divide-y divide-hairline text-foreground" data-testid="appearance-settings">
  {#if saveError}
    <p role="alert" class="my-3 rounded-lg border border-destructive/40 bg-destructive/10 px-3 py-2 text-xs text-destructive">
      {m.system_appearance_save_error({ error: saveError })}
    </p>
  {/if}

  <div class="flex flex-wrap items-center justify-between gap-x-6 gap-y-2 py-3.5">
    <div class="min-w-0">
      <p class="m-0 flex items-center gap-1.5 text-[13px] font-medium"><SunMoon class="size-3.5 text-muted-foreground" />{m.system_appearance_mode_title()}</p>
      <p class="mt-0.5 mb-0 text-xs text-muted-foreground">{m.system_appearance_mode_description()}</p>
    </div>
    <div class="flex rounded-lg bg-muted/70 p-0.5" role="group" aria-label={m.system_appearance_mode_title()}>
      {#each THEME_MODES as mode (mode.id)}
        {@const Icon = modeIcons[mode.id]}
        {@const active = $appearancePreferences.mode === mode.id}
        <button type="button" data-testid={`appearance-mode-${mode.id}`} aria-pressed={active}
          class={["flex h-7 items-center gap-1.5 rounded-md px-3 text-xs transition-colors focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring",
            active ? "bg-background font-medium text-foreground shadow-sm" : "text-muted-foreground hover:text-foreground"]}
          onclick={() => save({ mode: mode.id as ThemeMode })}>
          <Icon class="size-3.5" />{mode.label}
        </button>
      {/each}
    </div>
  </div>

  <div class="py-3.5">
    <p class="m-0 flex items-center gap-1.5 text-[13px] font-medium"><Palette class="size-3.5 text-muted-foreground" />{m.system_appearance_palette_title()}</p>
    <p class="mt-0.5 mb-2.5 text-xs text-muted-foreground">{m.system_appearance_palette_description()}</p>
    <div class="grid grid-cols-3 gap-2 sm:grid-cols-4 lg:grid-cols-5" role="group" aria-label={m.system_appearance_palette_title()}>
      {#each PALETTES as palette (palette.id)}
        {@const active = $appearancePreferences.palette === palette.id}
        <button type="button" data-testid={`appearance-palette-${palette.id}`} aria-pressed={active}
          class={["flex h-9 min-w-0 items-center gap-2 rounded-lg border px-3 text-left text-xs transition-colors hover:bg-muted/60 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring",
            active ? "border-primary bg-primary/8 ring-2 ring-primary/20" : "border-hairline"]}
          onclick={() => save({ palette: palette.id })}>
          <span class="size-4 shrink-0 rounded-full border border-foreground/15" style:background-color={palette.swatch}></span>
          <span class="truncate">{palette.label}</span>
        </button>
      {/each}
    </div>
  </div>

  <div class="flex flex-wrap items-center justify-between gap-x-6 gap-y-2 py-3.5">
    <div class="min-w-0">
      <p class="m-0 flex items-center gap-1.5 text-[13px] font-medium"><Scaling class="size-3.5 text-muted-foreground" />{m.system_appearance_zoom_title()}</p>
      <p class="mt-0.5 mb-0 text-xs text-muted-foreground">{m.system_appearance_zoom_description()}</p>
    </div>
    <div class="w-48">
      <Select.Root type="single" value={String($appearancePreferences.zoom)} onValueChange={(value) => save({ zoom: Number(value) })}>
        <Select.Trigger class="w-full bg-background/60 text-[13px]" aria-label={m.system_appearance_zoom_title()} data-testid="appearance-zoom">{$appearancePreferences.zoom}%{$appearancePreferences.zoom === 100 ? m.system_common_default_suffix() : ""}</Select.Trigger>
        <Select.Content class="ui-scope">
          {#each ZOOM_LEVELS as zoom (zoom)}
            <Select.Item value={String(zoom)} label={`${zoom}%${zoom === 100 ? m.system_common_default_suffix() : ""}`} />
          {/each}
        </Select.Content>
      </Select.Root>
    </div>
  </div>

  <div class="py-3.5">
    <div class="flex flex-wrap items-center justify-between gap-x-6 gap-y-2">
      <div class="min-w-0">
        <p class="m-0 flex items-center gap-1.5 text-[13px] font-medium"><Type class="size-3.5 text-muted-foreground" />{m.system_appearance_font_title()}</p>
        <p class="mt-0.5 mb-0 text-xs text-muted-foreground">{m.system_appearance_font_description()}</p>
      </div>
      <div class="w-72 max-w-full">
        <Select.Root type="single" value={$appearancePreferences.uiFont} onValueChange={(value) => save({ uiFont: value as FontId })}>
          <Select.Trigger class="w-full bg-background/60 text-[13px]" aria-label={m.system_appearance_font_title()} data-testid="appearance-font">{UI_FONTS.find((font) => font.id === $appearancePreferences.uiFont)?.label}</Select.Trigger>
          <Select.Content class="ui-scope">
            {#each UI_FONTS as font (font.id)}
              <Select.Item value={font.id} label={font.label} />
            {/each}
          </Select.Content>
        </Select.Root>
      </div>
    </div>
    {#if $appearancePreferences.uiFont === "custom"}
      <div class="mt-3 space-y-1.5">
        <label for="appearance-custom-font" class="text-xs font-medium text-muted-foreground">{m.system_appearance_custom_font_label()}</label>
        <Input id="appearance-custom-font" bind:value={customFont} placeholder={m.system_appearance_custom_font_placeholder()} maxlength={160}
          onchange={() => save({ uiCustomFont: customFont })} />
        <p class="m-0 text-[11px] text-muted-foreground">{m.system_appearance_custom_font_hint()}</p>
      </div>
    {/if}
    <p class="mt-3 mb-0 rounded-lg border border-hairline bg-background/50 px-3 py-2 text-sm" style:font-family={fontFamily} data-testid="appearance-preview">
      {m.system_appearance_font_preview()}
    </p>
  </div>

  <div class="flex flex-wrap items-center gap-3 py-3.5">
    <Button variant="outline" size="sm" data-testid="appearance-reset" onclick={reset}><RotateCcw class="size-3.5" />{m.system_appearance_reset_button()}</Button>
    {#if restored}<span role="status" class="text-xs text-muted-foreground">{m.system_appearance_reset_done()}</span>{/if}
  </div>
  <div class="flex flex-wrap items-center justify-between gap-x-6 gap-y-2 py-3.5">
    <div class="min-w-0">
      <p class="m-0 flex items-center gap-1.5 text-[13px] font-medium"><Languages class="size-3.5 text-muted-foreground" />{m.locale_title()}</p>
      <p class="mt-0.5 mb-0 text-xs text-muted-foreground">{m.locale_description()}</p>
    </div>
    <div class="flex rounded-lg bg-muted/70 p-0.5" role="group" aria-label={m.locale_title()}>
      {#each LANGUAGES as language (language.value)}
        {@const active = currentLocale() === language.value}
        <button type="button" data-testid={`appearance-locale-${language.value}`} aria-pressed={active}
          class={["flex h-7 items-center rounded-md px-3 text-xs transition-colors focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring",
            active ? "bg-background font-medium text-foreground shadow-sm" : "text-muted-foreground hover:text-foreground"]}
          onclick={() => void changeLocale(language.value)}>
          {language.label}
        </button>
      {/each}
    </div>
  </div>
</div>
