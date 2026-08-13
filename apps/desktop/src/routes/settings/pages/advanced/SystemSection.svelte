<script lang="ts">
  import { onMount } from "svelte";
  import { save as saveDialog } from "@tauri-apps/plugin-dialog";
  import {
    exportDiagnostics,
    getElevationStatus,
    getHotkeyStatus,
    isTauri,
    restartAsAdmin,
    type ElevationStatus,
    type HotkeyStatus,
  } from "../../../../lib/ipc";
  import { errText, patchSettings, settingsStore, toast } from "../../../../lib/stores/ui";
  import Toggle from "../../../../lib/components/Toggle.svelte";

  let { onOpenOnboarding }: { onOpenOnboarding: () => void } = $props();

  const ADMIN_RESTART_FLAG = "kotone:admin-restart-pending";

  let elevation = $state<ElevationStatus | null>(null);
  let hotkeyStatus = $state<HotkeyStatus | null>(null);
  let restartingAsAdmin = $state(false);
  let exportingDiagnostics = $state(false);

  onMount(async () => {
    [elevation, hotkeyStatus] = await Promise.all([
      getElevationStatus().catch(() => null),
      getHotkeyStatus().catch(() => null),
    ]);
    if (localStorage.getItem(ADMIN_RESTART_FLAG)) {
      localStorage.removeItem(ADMIN_RESTART_FLAG);
      if (elevation?.elevated) toast(true, "已通过管理员权限运行");
    }
  });

  async function onHotkeyBackendChange(e: Event) {
    const hotkeyBackend = (e.target as HTMLSelectElement).value;
    await patchSettings({ hotkeyBackend }, "热键兼容模式已更新");
    hotkeyStatus = await getHotkeyStatus().catch(() => hotkeyStatus);
  }

  async function onExportDiagnostics() {
    if (exportingDiagnostics) return;
    exportingDiagnostics = true;
    try {
      const defaultName = `kotone-diagnostics-${new Date().toISOString().slice(0, 10)}.zip`;
      const path = isTauri
        ? await saveDialog({
            title: "导出 Kotone 诊断包",
            defaultPath: defaultName,
            filters: [{ name: "ZIP 诊断包", extensions: ["zip"] }],
          })
        : defaultName;
      if (!path) return;
      const result = await exportDiagnostics(path);
      toast(true, `诊断包已导出：${result.reportId}`);
    } catch (error) {
      toast(false, `导出诊断包失败：${errText(error)}`);
    } finally {
      exportingDiagnostics = false;
    }
  }

  async function onRestartAsAdmin() {
    restartingAsAdmin = true;
    try {
      localStorage.setItem(ADMIN_RESTART_FLAG, "1");
      await restartAsAdmin();
    } catch (error) {
      localStorage.removeItem(ADMIN_RESTART_FLAG);
      restartingAsAdmin = false;
      toast(false, errText(error));
    }
  }
</script>

{#if $settingsStore}
  <section class="kotone-panel flex items-center justify-between gap-4 p-4">
    <div>
      <h2 class="text-sm font-semibold text-kotone-cyan/90">语言</h2>
      <p class="mt-1 text-[11px] leading-relaxed text-white/50">界面语言。更多语言即将到来。</p>
    </div>
    <select
      class="shrink-0 rounded-lg bg-white/8 px-2.5 py-1.5 text-xs ring-1 ring-white/15 outline-none focus:ring-kotone-cyan/60 [&>option]:bg-kotone-deep"
      value={$settingsStore.language}
      onchange={(e) =>
        void patchSettings({ language: (e.target as HTMLSelectElement).value }, "语言已切换")}
    >
      <option value="zh">中文</option>
    </select>
  </section>

  <section class="kotone-panel mt-3 flex flex-col gap-4 p-4">
    <h2 class="text-sm font-semibold text-kotone-cyan/90">运行时</h2>
    <Toggle
      checked={$settingsStore.ui.autoStart}
      label="启动 Kotone 后自动开始运行"
      desc="自动加载识别模型、注册热键并显示悬浮窗；关闭时需手动点「启动」"
      onchange={(v) =>
        void patchSettings({ ui: { autoStart: v } }, v ? "已开启自动启动" : "已关闭自动启动，需手动点「启动」")}
    />
  </section>

  <section class="kotone-panel mt-3 flex items-center justify-between gap-4 p-4">
    <div>
      <h2 class="text-sm font-semibold text-kotone-cyan/90">设置助手</h2>
      <p class="mt-1 text-[11px] leading-relaxed text-white/50">
        重新选择游戏配置、检查模型与麦克风、设置热键，并完成一次真实发送测试。
      </p>
    </div>
    <button
      class="shrink-0 rounded-lg bg-white/10 px-3 py-2 text-xs font-semibold text-white/85 ring-1 ring-white/15 transition hover:bg-white/20 focus-visible:ring-2 focus-visible:ring-kotone-cyan"
      onclick={onOpenOnboarding}
    >
      重新运行向导
    </button>
  </section>

  <section class="kotone-panel mt-3 p-4">
    <h2 class="text-sm font-semibold text-kotone-cyan/90">悬浮窗外观</h2>
    <p class="mt-1 text-[11px] text-white/45">只改变展示密度，不影响识别和发送。</p>
    <div class="mt-3 grid grid-cols-2 gap-2">
      {#each [
        { id: "capsule", name: "胶囊", desc: "更轻巧，减少遮挡" },
        { id: "card", name: "卡片", desc: "信息完整，适合首次使用" },
      ] as option}
        {@const selected = $settingsStore.overlay.style === option.id}
        <button
          class="rounded-lg px-3 py-2.5 text-left ring-1 transition {selected
            ? 'bg-kotone-violet/15 ring-kotone-violet/60'
            : 'bg-white/5 ring-white/10 hover:bg-white/10'}"
          onclick={() =>
            void patchSettings({ overlay: { style: option.id } }, `悬浮窗外观已切换：${option.name}`)}
        >
          <p class="text-sm font-semibold {selected ? 'text-kotone-violet' : ''}">{option.name}</p>
          <p class="mt-0.5 text-[11px] text-white/45">{option.desc}</p>
        </button>
      {/each}
    </div>
  </section>

  <section class="kotone-panel mt-3 p-4">
    <h2 class="text-sm font-semibold text-kotone-cyan/90">Windows 兼容性</h2>
    <div class="mt-3 flex items-center gap-2">
      <label class="text-xs text-white/60" for="advanced-hotkey-backend">热键实现</label>
      <select
        id="advanced-hotkey-backend"
        class="rounded-lg bg-white/8 px-2.5 py-1.5 text-xs ring-1 ring-white/15 outline-none focus:ring-kotone-cyan/60 [&>option]:bg-kotone-deep"
        value={$settingsStore.hotkeyBackend}
        onchange={(event) => void onHotkeyBackendChange(event)}
      >
        <option value="auto">自动（推荐）</option>
        <option value="llhook">低层键鼠钩子</option>
        <option value="register">系统热键</option>
      </select>
      <span class="text-[11px] text-white/40">
        当前：{hotkeyStatus?.backend === "llhook"
          ? "低层钩子"
          : hotkeyStatus?.backend === "register"
            ? "系统热键"
            : "未注册"}
      </span>
    </div>
    <p class="mt-2 text-[11px] text-white/40">除非特定游戏收不到热键，否则保持“自动”。</p>

    <div class="mt-4 border-t border-white/8 pt-4">
      <div class="flex items-center justify-between">
        <div>
          <p class="text-xs font-semibold">当前进程权限</p>
          <p class="mt-0.5 text-[11px] text-white/40">
            {elevation === null ? "检测中…" : elevation.elevated ? "管理员" : "普通用户"}
          </p>
        </div>
        {#if elevation && !elevation.elevated}
          <button
            class="rounded-lg bg-white/10 px-3 py-1.5 text-xs font-semibold text-white/75 hover:bg-white/20 disabled:opacity-50"
            disabled={restartingAsAdmin}
            onclick={() => void onRestartAsAdmin()}
          >
            {restartingAsAdmin ? "正在重启…" : "以管理员身份重启"}
          </button>
        {/if}
      </div>
      <div class="mt-4">
        <Toggle
          checked={$settingsStore.runAsAdminOnStart}
          label="启动时自动请求管理员权限"
          desc="开启后每次启动都会自动发起 Windows UAC 确认；普通桌面应用不能静默获得管理员权限"
          onchange={(value) =>
            void patchSettings(
              {
                runAsAdminOnStart: value,
                ...(value ? { adminPromptDismissed: true, autoAdminPromptDismissed: true } : {}),
              },
              value ? "已开启启动时自动权限请求" : "已关闭启动时自动权限请求",
            )}
        />
      </div>
    </div>
  </section>

  <section class="kotone-panel mt-3 p-4">
    <h2 class="text-sm font-semibold text-kotone-cyan/90">诊断包</h2>
    <p class="mt-1 text-[11px] text-white/45">
      不包含录音、识别文本和热词，可安全分享给测试群管理员。
    </p>
    <button
      class="mt-3 inline-flex items-center gap-2 rounded-xl bg-kotone-cyan/12 px-4 py-2.5 text-xs font-semibold text-kotone-cyan
        ring-1 ring-kotone-cyan/25 transition hover:bg-kotone-cyan/18 hover:shadow-glow-cyan
        disabled:cursor-wait disabled:opacity-60"
      disabled={exportingDiagnostics}
      onclick={() => void onExportDiagnostics()}
    >
      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="h-4 w-4" aria-hidden="true">
        <path d="M12 3v12m0 0 4-4m-4 4-4-4M5 18v2h14v-2" stroke-linecap="round" stroke-linejoin="round"/>
      </svg>
      {exportingDiagnostics ? "正在生成…" : "导出诊断包"}
    </button>
  </section>
{/if}
