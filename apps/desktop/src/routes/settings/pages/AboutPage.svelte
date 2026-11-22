<script lang="ts">
  /*
   * 关于页（单卡片版）：一张项目卡片——左侧贴纸，右侧版本号、GitHub
   * 地址与「角色详情」入口（→ CharacterPage 全屏档案）。
   * 完整角色内容（立绘 / 故事 / 贴纸 / 海报）都在角色详情页；
   * 导出诊断包在「高级」页。
   */
  import { onMount } from "svelte";
  import { getVersion } from "@tauri-apps/api/app";
  import { isTauri, openExternal } from "../../../lib/ipc";
  import { toast, errText, patchSettings, settingsStore } from "../../../lib/stores/ui";
  import { checkForUpdates, type UpdateCheckResult } from "../../../lib/updater";
  import stickerProud from "../../../assets/brand/stickers/proud.webp";
  import SettingsPage from "../../../lib/components/settings/SettingsPage.svelte";
  import SettingsSection from "../../../lib/components/settings/SettingsSection.svelte";
  import SettingsRow from "../../../lib/components/settings/SettingsRow.svelte";
  import { Switch } from "$lib/components/ui/switch";
  import { Button } from "$lib/components/ui/button";
  import { ChevronRight, RefreshCw } from "@lucide/svelte";
  import { m } from "$lib/paraglide/messages.js";

  /** 角色详情页入口回调：由 Settings 切换至 CharacterPage 全屏档案视图 */
  let { onOpenCharacter }: { onOpenCharacter: () => void } = $props();

  /** 兜底为 build-time 注入的 package.json 版本；桌面端启动后替换为运行时真实版本 */
  let version = $state(__APP_VERSION__);
  let checkingUpdate = $state(false);
  let updateResult = $state<UpdateCheckResult | null>(null);

  onMount(async () => {
    if (!isTauri) return;
    version = await getVersion().catch(() => version);
  });

  const GITHUB_URL = "https://github.com/l1veIn/kotone";

  /** webview 内 target=_blank 不会调起系统浏览器，走后端 open_external */
  async function openGitHub(e: MouseEvent) {
    e.preventDefault();
    try {
      await openExternal(GITHUB_URL);
    } catch (err) {
      toast(false, m.system_about_open_link_failed({ error: errText(err) }));
    }
  }

  async function checkUpdates() {
    if (checkingUpdate) return;
    checkingUpdate = true;
    updateResult = null;
    try {
      updateResult = await checkForUpdates();
    } finally {
      checkingUpdate = false;
    }
  }
</script>

<SettingsPage title={m.system_about_title()} description={m.system_about_description()} testid="about-page">
  <!-- 项目卡：琴音贴纸 + 名称、版本、链接与角色详情入口 -->
  <section class="relative overflow-hidden rounded-2xl border border-hairline bg-card px-5 py-5">
    <div class="pointer-events-none absolute -top-16 -right-10 size-56 rounded-full bg-primary/10 blur-3xl" aria-hidden="true"></div>
    <div class="pointer-events-none absolute -bottom-20 left-10 size-48 rounded-full bg-brand-pink/8 blur-3xl" aria-hidden="true"></div>
    <div class="relative flex flex-wrap items-center gap-5">
      <img src={stickerProud} alt={m.system_about_sticker_alt()} class="size-24 shrink-0 object-contain drop-shadow-[0_0_18px_color-mix(in_oklab,var(--color-primary)_35%,transparent)]" />
      <div class="min-w-0 flex-1">
        <p class="m-0 flex flex-wrap items-center gap-2 text-lg font-bold text-foreground">
          Kotone <span class="kotone-gradient-text">{m.system_about_brand_name()}</span>
          <span class="rounded-full border border-hairline bg-muted/60 px-2 py-0.5 text-[11px] font-normal text-muted-foreground">v{version}</span>
        </p>
        <p class="mt-1 mb-0 text-xs text-muted-foreground">{m.system_about_tagline()}</p>
        <div class="mt-3 flex flex-wrap items-center gap-2">
          <a
            class="inline-flex h-8 items-center gap-1.5 rounded-lg border border-hairline bg-background/50 px-3 text-xs text-foreground/85 transition-colors hover:bg-muted hover:text-primary"
            href={GITHUB_URL}
            target="_blank"
            rel="noreferrer"
            onclick={(e) => void openGitHub(e)}
          >
            <svg viewBox="0 0 24 24" fill="currentColor" class="size-3.5" aria-hidden="true">
              <path
                d="M12 .5C5.65.5.5 5.65.5 12c0 5.08 3.29 9.39 7.86 10.91.58.11.79-.25.79-.56 0-.27-.01-1.17-.02-2.12-3.2.7-3.88-1.36-3.88-1.36-.52-1.33-1.28-1.68-1.28-1.68-1.04-.71.08-.7.08-.7 1.15.08 1.76 1.18 1.76 1.18 1.03 1.76 2.69 1.25 3.35.96.1-.75.4-1.25.72-1.54-2.55-.29-5.23-1.28-5.23-5.68 0-1.26.45-2.28 1.18-3.09-.12-.29-.51-1.46.11-3.05 0 0 .96-.31 3.15 1.18a10.9 10.9 0 0 1 5.74 0c2.19-1.49 3.15-1.18 3.15-1.18.62 1.59.23 2.76.11 3.05.74.81 1.18 1.83 1.18 3.09 0 4.41-2.69 5.38-5.25 5.66.41.35.77 1.05.77 2.12 0 1.53-.01 2.76-.01 3.14 0 .31.21.68.8.56A10.52 10.52 0 0 0 23.5 12C23.5 5.65 18.35.5 12 .5z"
              />
            </svg>
            github.com/l1veIn/kotone
          </a>
          <button
            class="group inline-flex h-8 items-center gap-1.5 rounded-lg bg-primary/12 px-3 text-xs font-semibold text-primary ring-1 ring-primary/35 transition-colors hover:bg-primary/20"
            onclick={onOpenCharacter}
          >
            {m.system_about_character_button()}
            <ChevronRight class="size-3.5 transition-transform group-hover:translate-x-0.5" />
          </button>
        </div>
      </div>
    </div>
  </section>

  <SettingsSection title={m.system_about_update_section()}>
    <SettingsRow label={m.system_about_check_updates()}>
      {#snippet description()}
        <div aria-live="polite">
          {#if checkingUpdate}
            <p class="m-0">{m.system_about_update_connecting()}</p>
          {:else if updateResult?.status === "up-to-date"}
            <p class="m-0 text-success">{m.system_about_update_up_to_date({ version })}</p>
          {:else if updateResult?.status === "available"}
            <p class="m-0 text-brand-pink">{m.system_about_update_available({ version: updateResult.version })}</p>
            {#if updateResult.notes}
              <p class="mt-1 mb-0 whitespace-pre-wrap break-words">{updateResult.notes}</p>
            {/if}
          {:else if updateResult?.status === "downloaded"}
            <p class="m-0 text-primary">{m.system_about_update_downloaded({ version: updateResult.version })}</p>
            {#if updateResult.notes}
              <p class="mt-1 mb-0 whitespace-pre-wrap break-words">{updateResult.notes}</p>
            {/if}
          {:else if updateResult?.status === "error"}
            <p class="m-0 break-all text-destructive">{m.system_about_update_check_failed({ error: updateResult.message })}</p>
          {:else}
            <p class="m-0">{m.system_about_update_idle()}</p>
          {/if}
        </div>
      {/snippet}
      {#snippet control()}
        <Button size="sm" variant="outline" data-testid="check-updates" disabled={checkingUpdate} onclick={() => void checkUpdates()}>
          <RefreshCw class="size-3.5 {checkingUpdate ? 'animate-spin' : ''}" />
          {checkingUpdate ? m.system_about_update_checking() : m.system_about_check_updates()}
        </Button>
      {/snippet}
    </SettingsRow>
    {#if $settingsStore}
      <SettingsRow label={m.system_about_update_prompt_label()} description={m.system_about_update_prompt_description()} for="update-prompt-enabled">
        {#snippet control()}
          <Switch
            id="update-prompt-enabled"
            data-testid="update-prompt-enabled"
            aria-label={m.system_about_update_prompt_label()}
            bind:checked={() => !$settingsStore.ui.updatePromptDisabled, (checked) =>
              void patchSettings(
                { ui: { updatePromptDisabled: !checked } },
                checked ? m.system_about_update_prompt_on() : m.system_about_update_prompt_off(),
              )}
          />
        {/snippet}
      </SettingsRow>
    {/if}
  </SettingsSection>

  <p class="m-0 text-center text-[11px] text-muted-foreground/70">
    {m.system_about_footer()}
  </p>
</SettingsPage>
