<script lang="ts">
  /*
   * 游戏适配页（瘦身版）：profile 游戏卡片（激活开关）+ 内联 profile 编辑器。
   * - 编辑器：发送键配置（openChatKey/sendKey）、聊天框模式（preferClipboardPaste）、
   *   热词管理（批量文本编辑：每行一个词条，与导入/导出同构 / 导出 / 导入）；
   * - 热词表格式：UTF-8 纯文本，每行一词条（底层 Vec<String>，无权重）；
   * - 导入为合并模式：跳过空行、精确匹配去重，报告「新增 N / 重复 M」；
   * - Tauri 走系统保存/打开对话框 + export_hotwords/import_hotwords IPC；
   *   dev:web 走 Blob 下载 / <input type=file> + 前端同款 parse/merge；
   * - 已删除：前台检测（detect_foreground_game IPC 已连删）、测试发送按钮
   *   （simulate_send IPC 保留——悬浮条错误重试在用）。
   * - 琴音与自动化：allowAutomationInput（缺省开）、computerUse（让琴音操作，缺省先问）、
   *   launchPath（琴音打开游戏用）；
   * - 控制台通道（ADR 032 决定 3）：profile 声明了 consoleChat 才显示开关。开关不随“保存”走，
   *   一拨就改游戏目录里的 autoexec.cfg（开关的状态就是那一行），游戏开着时不能拨；
   *   保存时整份 profile 原样带回，新字段不会丢。
   * - 版式：SettingsPage / SettingsSection（参照 DeepSeek Harness 设置页）。
   */
  import { companionName } from "$lib/stores/companion";
  import { onDestroy, onMount } from "svelte";
  import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
  import {
    updateSettings,
    listProfiles,
    saveProfile,
    exportHotwords,
    importHotwords,
    exportProfile,
    importProfile,
    deleteProfile,
    getProfileIcon,
    installedGames,
    consoleChatStatus,
    consoleChatSet,
    isTauri,
    type ConsoleChatStatus,
    type GameProfile,
    type InstalledGame,
  } from "../../../lib/ipc";
  import { settingsStore, toast, toastWarn, errText } from "../../../lib/stores/ui";
  import { captureHotkey } from "../../../lib/hotkeyCapture";
  import { Download, Gamepad2, Globe, LoaderCircle, ScanSearch, Upload } from "@lucide/svelte";
  import { Button } from "$lib/components/ui/button";
  import { Input } from "$lib/components/ui/input";
  import SettingsPage from "$lib/components/settings/SettingsPage.svelte";
  import SettingsSection from "$lib/components/settings/SettingsSection.svelte";
  import ToggleField from "$lib/components/settings/ToggleField.svelte";
  import SelectField from "$lib/components/settings/SelectField.svelte";
  import InstalledGamesSection from "./InstalledGamesSection.svelte";
  import { m } from "$lib/paraglide/messages.js";

  /** 让琴音操作（game.act）的三档。 */
  const COMPUTER_USE_OPTIONS = [
    { value: "ask", label: m.game_computer_use_ask(), description: m.game_computer_use_ask_desc() },
    { value: "allow", label: m.game_computer_use_allow(), description: m.game_computer_use_allow_desc() },
    { value: "deny", label: m.game_computer_use_deny(), description: m.game_computer_use_deny_desc() },
  ];

  let { legacyVoiceOnly = false }: { legacyVoiceOnly?: boolean } = $props();

  let profiles = $state<GameProfile[]>([]);
  /** 启动路径的“自动检测”结果（null = 还没检测）。 */
  let detected = $state<InstalledGame[] | null>(null);
  let detecting = $state(false);
  const SOURCE_LABEL: Record<string, string> = {
    steam: "Steam", tencent: m.game_source_tencent(), riot: "Riot", hoyoplay: m.game_source_hoyoplay(), epic: "Epic", wegame: "WeGame",
    uninstall: m.game_source_uninstall_system(),
  };

  /** 和当前游戏配置对得上的排前面：先比进程名，再比名字。 */
  function rankDetected(games: InstalledGame[], profile: GameProfile): InstalledGame[] {
    const norm = (s: string) => s.split(/[（(]/)[0].toLowerCase().replace(/[^\p{L}\p{N}]/gu, "");
    const procs = profile.processNames.map((p) => p.toLowerCase());
    const name = norm(profile.displayName);
    const score = (g: InstalledGame) =>
      g.processNames.some((p) => procs.includes(p.toLowerCase())) ? 2
        : name && (norm(g.name).includes(name) || name.includes(norm(g.name))) ? 1 : 0;
    return games.filter((g) => g.launch).sort((a, b) => score(b) - score(a));
  }

  async function detectGames() {
    if (detecting || !draft) return;
    detecting = true;
    try {
      detected = rankDetected(await installedGames(), draft);
      if (!detected.length) toast(false, m.game_detect_none());
    } catch (error) {
      toast(false, m.game_detect_failed({ error: errText(error) }));
    } finally {
      detecting = false;
    }
  }

  function useDetected(game: InstalledGame) {
    if (!draft || !game.launch) return;
    draft.launchPath = game.launch;
    detected = null;
    toast(true, m.game_detect_filled({ name: game.name }));
  }

  // ---------- 控制台通道（ADR 032 决定 3） ----------
  /** 编辑中的 profile 的控制台通道状态（null = 没声明或还在读） */
  let consoleStatus = $state<ConsoleChatStatus | null>(null);
  let consoleBusy = $state(false);

  async function loadConsoleStatus(p: GameProfile) {
    consoleStatus = null;
    if (!p.consoleChat) return;
    try {
      const next = await consoleChatStatus(p.id);
      if (editingId === p.id) consoleStatus = next;
    } catch (error) {
      toast(false, m.game_console_status_failed({ error: errText(error) }));
    }
  }

  async function toggleConsole(p: GameProfile, enabled: boolean) {
    if (consoleBusy) return;
    consoleBusy = true;
    try {
      consoleStatus = await consoleChatSet(p.id, enabled);
      toast(
        true,
        enabled
          ? m.game_console_enabled_toast()
          : m.game_console_disabled_toast(),
      );
    } catch (error) {
      toast(false, errText(error));
      await loadConsoleStatus(p);
    } finally {
      consoleBusy = false;
    }
  }

  // ---------- 内联编辑器状态 ----------
  /** 展开编辑中的 profile id（null = 全部收起） */
  let editingId = $state<string | null>(null);
  /** 编辑草稿（$state.snapshot 取纯数据深拷贝——$state 数组元素是 Proxy，
   *  structuredClone 不认会抛 DataCloneError；保存才落盘） */
  let draft = $state<GameProfile | null>(null);
  /** 热词批量编辑文本（每行一个词条，与导入/导出 txt 格式同构） */
  let hotwordsText = $state("");
  let saving = $state(false);
  let importing = $state(false);
  /** dev:web 导入用隐藏文件选择器 */
  let fileInput = $state<HTMLInputElement | null>(null);
  let importTargetId = $state<string | null>(null);
  let showTechnical = $state(false);
  type ProfileKeyField = "openChatKey" | "sendKey";
  /** 当前正在录入的聊天键；全页共用底层捕获槽，不能同时录两个。 */
  let capturingKey = $state<ProfileKeyField | null>(null);
  let captureCleanup: (() => void) | null = null;
  /** 整包导入（.zip 配置包）与删除 */
  let importingPackage = $state(false);
  let deletingId = $state<string | null>(null);
  /** dev:web 导入 .zip 配置包的隐藏文件选择器 */
  let pkgFileInput = $state<HTMLInputElement | null>(null);
  /** profile 图标 blob URL 缓存（profile id → URL；空串 = 无图标） */
  const iconUrls = $state<Record<string, string>>({});
  const loadingIcons = $state<Set<string>>(new Set());

  /** 实时词条数：非空 trim 行数（不去重——去重在保存时做并提示） */
  const hotwordCount = $derived(
    hotwordsText.split(/\r?\n/).filter((l) => l.trim() !== "").length,
  );

  /** LOL 和通用配置支持恢复出厂。Dota 2 删除后不随升级装回。 */
  function isBuiltin(p: GameProfile): boolean {
    return p.id === "lol" || p.id === "generic";
  }

  /** 加载 profile 图标字节 → data URL（按扩展名推断 mime；无图标/失败 → 占位）。
   *  用 data: 而非 blob:——CSP img-src 只放行 'self' data: asset:，blob: 会被拦截不显示。 */
  async function ensureIcon(p: GameProfile) {
    if (!p.icon || iconUrls[p.id] !== undefined || loadingIcons.has(p.id)) return;
    loadingIcons.add(p.id);
    try {
      const bytes = await getProfileIcon(p.id);
      if (bytes.length === 0) {
        iconUrls[p.id] = "";
        return;
      }
      const ext = (p.icon.split(".").pop() ?? "webp").toLowerCase();
      const mime =
        ext === "png" ? "image/png" : ext === "jpg" || ext === "jpeg" ? "image/jpeg" : "image/webp";
      iconUrls[p.id] = `data:${mime};base64,${bytesToBase64(bytes)}`;
    } catch {
      iconUrls[p.id] = "";
    } finally {
      loadingIcons.delete(p.id);
    }
  }

  /** Uint8Array → base64（图标 ≤ 数十 KB，直接逐字节拼即可） */
  function bytesToBase64(bytes: Uint8Array): string {
    let binary = "";
    for (let i = 0; i < bytes.length; i++) binary += String.fromCharCode(bytes[i]);
    return btoa(binary);
  }

  /** 拉取 profile 列表并顺带加载各图标 */
  async function loadProfiles() {
    profiles = await listProfiles();
    for (const p of profiles) void ensureIcon(p);
  }

  onMount(async () => {
    try {
      await loadProfiles();
    } catch (e) {
      toast(false, m.game_profiles_load_failed({ error: errText(e) }));
    }
  });

  async function onActivate(id: string) {
    try {
      settingsStore.set(await updateSettings({ activeProfileId: id }));
      toast(true, m.game_profile_activated({ name: profiles.find((p) => p.id === id)?.displayName ?? id }));
    } catch (e) {
      toast(false, m.game_save_failed({ error: errText(e) }));
    }
  }

  // ---------- 编辑器 ----------

  function openEditor(p: GameProfile) {
    editingId = p.id;
    draft = $state.snapshot(p);
    hotwordsText = p.hotwords.join("\n");
    showTechnical = false;
    void loadConsoleStatus(p);
  }

  function closeEditor() {
    captureCleanup?.();
    captureCleanup = null;
    capturingKey = null;
    editingId = null;
    draft = null;
    hotwordsText = "";
    showTechnical = false;
    consoleStatus = null;
  }

  async function captureProfileKey(field: ProfileKeyField) {
    if (!draft || capturingKey) return;
    capturingKey = field;
    const cleanup = await captureHotkey((result) => {
      captureCleanup = null;
      capturingKey = null;
      if (result.kind === "combo" && draft) {
        draft[field] = result.combo;
      } else if (result.kind === "timeout") {
        toastWarn(m.game_key_capture_timeout());
      } else if (result.kind === "error") {
        toast(false, result.message);
      }
    });
    // 录入初始化期间编辑器可能已被关闭；此时立刻释放底层捕获槽，避免页面外残留监听。
    if (capturingKey !== field || !draft) {
      cleanup();
      return;
    }
    captureCleanup = cleanup;
  }

  function clearProfileKey(field: ProfileKeyField) {
    if (capturingKey || !draft) return;
    draft[field] = "";
  }

  onDestroy(() => captureCleanup?.());

  async function onSave() {
    if (!draft || saving) return;
    saving = true;
    try {
      // 热词规范化：与 core parse_hotwords_import 同款规则
      // （trim、去空行、保序去重）；有重复被合并则提示
      const normalized = parseImport(hotwordsText);
      const duplicates = hotwordCount - normalized.length;
      const next = { ...$state.snapshot(draft), hotwords: normalized };
      await saveProfile(next);
      profiles = profiles.map((p) => (p.id === next.id ? next : p));
      toast(
        true,
        duplicates > 0
          ? m.game_profile_saved_merged({ count: duplicates })
          : m.game_profile_saved(),
      );
      closeEditor();
    } catch (e) {
      toast(false, m.game_save_failed({ error: errText(e) }));
    } finally {
      saving = false;
    }
  }

  // ---------- 热词导入导出 ----------

  /** 与 core profile::format_hotwords_export 同款：每行一词条，末尾换行，空表→空串 */
  function formatExport(list: string[]): string {
    return list.length === 0 ? "" : list.join("\n") + "\n";
  }

  /** 与 core profile::parse_hotwords_import 同款：trim、跳空行、文件内去重保序 */
  function parseImport(text: string): string[] {
    const seen = new Set<string>();
    const out: string[] = [];
    for (const line of text.split(/\r?\n/)) {
      const w = line.trim();
      if (!w || seen.has(w)) continue;
      seen.add(w);
      out.push(w);
    }
    return out;
  }

  /** 与 core profile::merge_hotwords 同款：精确匹配去重，新增追加末尾 */
  function merge(existing: string[], incoming: string[]) {
    const set = new Set(existing);
    let added = 0;
    const merged = [...existing];
    for (const w of incoming) {
      if (set.has(w)) continue;
      set.add(w);
      merged.push(w);
      added++;
    }
    return { merged, added, duplicates: incoming.length - added };
  }

  /** 导入后刷新 profile 列表；若编辑器开着同一 profile，同步编辑器里的热词文本 */
  async function refreshAfterImport(profileId: string) {
    await loadProfiles();
    if (editingId === profileId && draft) {
      const fresh = profiles.find((p) => p.id === profileId);
      if (fresh) hotwordsText = fresh.hotwords.join("\n");
    }
  }

  async function onExport(p: GameProfile) {
    try {
      if (isTauri) {
        const path = await saveDialog({
          defaultPath: `${p.id}-hotwords.txt`,
          filters: [{ name: m.game_hotwords_file_filter(), extensions: ["txt"] }],
        });
        if (!path) return; // 用户取消，不打扰
        const n = await exportHotwords(p.id, path);
        toast(true, m.game_hotwords_exported_to({ count: n, path }));
      } else {
        const blob = new Blob([formatExport(p.hotwords)], { type: "text/plain;charset=utf-8" });
        const url = URL.createObjectURL(blob);
        const a = document.createElement("a");
        a.href = url;
        a.download = `${p.id}-hotwords.txt`;
        a.click();
        URL.revokeObjectURL(url);
        toast(true, m.game_hotwords_exported({ count: p.hotwords.length }));
      }
    } catch (e) {
      toast(false, m.game_export_failed({ error: errText(e) }));
    }
  }

  async function onImport(p: GameProfile) {
    if (importing) return;
    if (isTauri) {
      try {
        const sel = await openDialog({
          multiple: false,
          filters: [{ name: m.game_hotwords_file_filter(), extensions: ["txt"] }],
        });
        if (!sel || typeof sel !== "string") return; // 用户取消
        importing = true;
        const report = await importHotwords(p.id, sel);
        await refreshAfterImport(p.id);
        toast(true, m.game_hotwords_imported({ added: report.added, duplicates: report.duplicates }));
      } catch (e) {
        toast(false, m.game_import_failed({ error: errText(e) }));
      } finally {
        importing = false;
      }
    } else {
      // dev:web：隐藏 file input 读文本，前端复刻 parse/merge 后直接 saveProfile
      importTargetId = p.id;
      fileInput?.click();
    }
  }

  async function onImportFile(e: Event) {
    const input = e.target as HTMLInputElement;
    const file = input.files?.[0];
    input.value = ""; // 允许重复选同一文件
    const p = profiles.find((x) => x.id === importTargetId);
    importTargetId = null;
    if (!file || !p || importing) return;
    importing = true;
    try {
      const text = await file.text();
      const incoming = parseImport(text);
      const { merged, added, duplicates } = merge(p.hotwords, incoming);
      await saveProfile({ ...p, hotwords: merged });
      await refreshAfterImport(p.id);
      toast(true, m.game_hotwords_imported({ added, duplicates }));
    } catch (err) {
      toast(false, m.game_import_failed({ error: errText(err) }));
    } finally {
      importing = false;
    }
  }

  // ---------- 整包导入导出（.zip：profile.json + icon） ----------

  /** 导出整包：含 icon，供分享/编辑成别的游戏再导入 */
  async function onExportPackage(p: GameProfile) {
    if (!isTauri) {
      toastWarn(m.game_package_export_unsupported());
      return;
    }
    try {
      const path = await saveDialog({
        defaultPath: `${p.displayName}.zip`,
        filters: [{ name: m.game_package_file_filter(), extensions: ["zip"] }],
      });
      if (!path) return; // 用户取消
      await exportProfile(p.id, path);
      toast(true, m.game_package_exported({ path }));
    } catch (e) {
      toast(false, m.game_export_failed({ error: errText(e) }));
    }
  }

  /** 导入整包（生成新 id，用包名区分），成功后刷新列表 */
  async function onImportPackage() {
    if (!isTauri) {
      pkgFileInput?.click();
      return;
    }
    if (importingPackage) return;
    try {
      const sel = await openDialog({
        multiple: false,
        filters: [{ name: m.game_package_file_filter(), extensions: ["zip"] }],
      });
      if (!sel || typeof sel !== "string") return; // 用户取消
      importingPackage = true;
      const imported = await importProfile(sel);
      await loadProfiles();
      toast(true, m.game_package_imported({ name: imported.displayName }));
    } catch (e) {
      toast(false, m.game_import_failed({ error: errText(e) }));
    } finally {
      importingPackage = false;
    }
  }

  /** dev:web 导入 .zip：读文件名 → mock importProfile（无法解析真实 zip） */
  async function onImportPkgFile(e: Event) {
    const input = e.target as HTMLInputElement;
    const file = input.files?.[0];
    input.value = ""; // 允许重复选同一文件
    if (!file) return;
    try {
      const imported = await importProfile(file.name);
      await loadProfiles();
      toast(true, m.game_package_imported_mock({ name: imported.displayName }));
    } catch (err) {
      toast(false, m.game_import_failed({ error: errText(err) }));
    }
  }

  /** 删除/恢复：内置 = 恢复出厂；导入的 = 永久删除。二次点击确认（3s 超时取消） */
  async function onDeleteProfile(p: GameProfile) {
    if (deletingId !== p.id) {
      deletingId = p.id;
      setTimeout(() => {
        if (deletingId === p.id) deletingId = null;
      }, 3000);
      return;
    }
    deletingId = null;
    try {
      const kind = await deleteProfile(p.id);
      // 删除的是当前激活 profile → 清 activeProfileId（orchestrator 兜底 generic）
      if ($settingsStore?.activeProfileId === p.id) {
        settingsStore.set(await updateSettings({ activeProfileId: null }));
      }
      await loadProfiles();
      toast(
        true,
        kind === "reset" ? m.game_profile_reset_done({ name: p.displayName }) : m.game_profile_deleted({ name: p.displayName }),
      );
    } catch (e) {
      toast(false, m.game_action_failed({ error: errText(e) }));
    }
  }
</script>

{#snippet field(label: string, hint: string = "")}
  <p class="m-0 text-xs font-medium text-foreground">{label}</p>
  {#if hint}<p class="mt-0.5 mb-0 text-[11px] leading-relaxed text-muted-foreground">{hint}</p>{/if}
{/snippet}

{#snippet slider(label: string, key: "preOpenDelayMs" | "prePasteDelayMs" | "preSendDelayMs")}
  {#if draft}
    <label class="block">
      <div class="flex items-center justify-between text-xs">
        <span class="text-foreground/85">{label}</span>
        <span class="text-muted-foreground tabular-nums">{draft[key]} ms</span>
      </div>
      <input type="range" min="0" max="300" step="10" bind:value={draft[key]} class="mt-1.5 w-full accent-primary" />
    </label>
  {/if}
{/snippet}

<SettingsPage title={legacyVoiceOnly ? m.game_page_title_legacy() : m.game_page_title()} description={legacyVoiceOnly ? m.game_page_description_legacy() : m.game_page_description()}>
  {#if !legacyVoiceOnly}
  <InstalledGamesSection {profiles} onOpenProfile={(p) => { openEditor(p); requestAnimationFrame(() => document.querySelector(`[data-testid="game-profile-${p.id}"]`)?.scrollIntoView({ block: "start" })); }} />
  {/if}
  {#snippet actions()}
    <Button variant="outline" size="sm" disabled={importingPackage} onclick={() => void onImportPackage()}>
      <Upload class="size-3.5" />{importingPackage ? m.game_importing() : m.game_package_import()}
    </Button>
  {/snippet}

  <SettingsSection title={m.game_profiles_title()} description={m.game_profiles_description()} flush>
    {#each profiles as p (p.id)}
      {@const active = $settingsStore?.activeProfileId === p.id}
      {@const editing = editingId === p.id && draft !== null}
      <div class="px-4 py-3.5" data-testid={`game-profile-${p.id}`}>
        <div class="flex flex-wrap items-center gap-3">
          <!-- 游戏图标：有 icon 用 profile 图标（IPC 加载 data URL），否则用占位 -->
          <span class="flex size-10 shrink-0 items-center justify-center overflow-hidden rounded-xl {active ? 'bg-primary/12 ring-1 ring-primary/40' : 'bg-muted'}">
            {#if p.icon && iconUrls[p.id]}
              <img src={iconUrls[p.id]} alt={p.displayName} class="h-full w-full object-contain" />
            {:else if p.processNames.length === 0}
              <Globe class="size-5 text-muted-foreground" />
            {:else}
              <Gamepad2 class="size-5 text-muted-foreground" />
            {/if}
          </span>
          <div class="min-w-0 flex-1">
            <p class="m-0 flex items-center gap-2 text-[13px] font-medium">
              <span class="truncate">{p.displayName}</span>
              {#if active}
                <span class="shrink-0 rounded-full bg-primary/12 px-2 py-0.5 text-[10px] font-medium text-primary">{m.game_profile_active()}</span>
              {/if}
            </p>
            <p class="mt-0.5 mb-0 truncate text-xs text-muted-foreground">
              {p.processNames.length > 0 ? m.game_profile_process_count({ count: p.processNames.length }) : m.game_profile_all_windows()}
              · {m.game_profile_hotword_count({ count: p.hotwords.length })}{p.allowAutomationInput === false ? ` · ${m.game_profile_no_automation_input()}` : ""}{p.computerUse === "allow" ? ` · ${m.game_profile_computer_use_allowed()}` : p.computerUse === "deny" ? ` · ${m.game_profile_computer_use_denied()}` : ""}
            </p>
          </div>
          <div class="flex shrink-0 flex-wrap items-center gap-1.5">
            <Button size="sm" variant={editing ? "secondary" : "outline"} onclick={() => (editing ? closeEditor() : openEditor(p))}>
              {editing ? m.game_profile_collapse() : m.game_profile_edit()}
            </Button>
            <Button size="sm" variant="ghost" title={m.game_package_export_title()} onclick={() => void onExportPackage(p)}>
              {m.game_export()}
            </Button>
            {#if !active}
              <Button size="sm" variant="outline" onclick={() => void onActivate(p.id)}>{m.game_profile_activate()}</Button>
            {/if}
            <Button size="sm" variant={deletingId === p.id ? "destructive" : "ghost"}
              class={deletingId === p.id ? "" : "text-muted-foreground"}
              title={isBuiltin(p) ? m.game_profile_reset_title() : m.game_profile_delete_title()}
              onclick={() => void onDeleteProfile(p)}>
              {deletingId === p.id
                ? isBuiltin(p)
                  ? m.game_profile_confirm_reset()
                  : m.game_profile_confirm_delete()
                : isBuiltin(p)
                  ? m.game_profile_reset()
                  : m.game_profile_delete()}
            </Button>
          </div>
        </div>

        {#if p.channels && p.channels.length > 1}
          <!-- 多频道声明（ADR-008）：写清当前适配支持哪些频道、按什么键切换 -->
          <div class="mt-3 flex flex-wrap items-center gap-1.5 pl-13">
            <span class="text-[11px] text-muted-foreground">{m.game_profile_channels()}</span>
            {#each p.channels as ch}
              <span
                class="rounded-full px-2 py-0.5 text-[11px] {ch.default ? 'bg-primary/12 text-primary' : 'bg-muted text-foreground/75'}"
                title={ch.textPrefix
                  ? m.game_channel_prefix_title({ prefix: ch.textPrefix })
                  : ch.openChatKey
                    ? m.game_channel_open_key_title({ key: ch.openChatKey })
                    : m.game_channel_default_key_title()}
              >
                {ch.displayName}{ch.default ? m.game_channel_default() : ""}
              </span>
            {/each}
            <span class="text-[11px] text-muted-foreground">
              · {m.game_channel_cycle_hint({ hotkey: $settingsStore?.channelCycleHotkey ?? "Shift+CapsLock" })}
            </span>
          </div>
        {/if}

        {#if editing && draft}
          <!-- 普通用户只编辑热词；聊天键和注入策略收进高级输入设置。 -->
          <div class="mt-4 space-y-5 rounded-xl border border-hairline bg-muted/30 p-4" data-testid="game-profile-editor">
            <div>
              <div class="flex items-center justify-between gap-2">
                {@render field(m.game_hotwords_label({ count: hotwordCount }))}
                <div class="flex shrink-0 gap-1.5">
                  <Button size="xs" variant="outline" disabled={importing} onclick={() => void onExport(p)}><Download class="size-3" />{m.game_export()}</Button>
                  <Button size="xs" variant="outline" disabled={importing} onclick={() => void onImport(p)}><Upload class="size-3" />{importing ? m.game_importing() : m.game_import()}</Button>
                </div>
              </div>
              <!-- 批量文本编辑：每行一个词条，与导入/导出 txt 格式同构（几百上千条可用） -->
              <textarea
                bind:value={hotwordsText}
                rows={10}
                placeholder={m.game_hotwords_placeholder()}
                spellcheck="false"
                class="mt-2 w-full resize-y rounded-lg border border-input bg-background/60 px-3 py-2 font-mono text-xs leading-relaxed text-foreground outline-none placeholder:text-muted-foreground focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/40"
              ></textarea>
              <p class="mt-1.5 mb-0 text-[11px] text-muted-foreground">
                {m.game_hotwords_help()}
              </p>
            </div>

            <div>
              {@render field(m.game_companion_notes_label())}
              <textarea
                bind:value={draft.companionNotes}
                rows={3}
                maxlength={500}
                data-testid="profile-companion-notes"
                placeholder={m.game_companion_notes_placeholder()}
                class="mt-2 w-full resize-y rounded-lg border border-input bg-background/60 px-3 py-2 text-xs leading-relaxed text-foreground outline-none placeholder:text-muted-foreground focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/40"
              ></textarea>
              <p class="mt-1.5 mb-0 text-[11px] text-muted-foreground">
                {m.game_companion_notes_help()}{draft.chatRegion ? m.game_companion_notes_chat_region() : ""}
              </p>
            </div>

            <!-- 琴音与自动化（游戏配置的新字段；保存时整份 profile 原样带回） -->
            <div class="space-y-4 rounded-lg border border-hairline bg-background/40 p-3.5" data-testid="game-profile-agent">
              <ToggleField
                checked={draft.allowAutomationInput ?? true}
                label={m.game_allow_automation_input()}
                desc={m.game_allow_automation_input_desc()}
                testid="profile-allow-automation-input"
                onchange={(v) => (draft!.allowAutomationInput = v)}
              />
              {#if !legacyVoiceOnly}
              <div class="flex items-start justify-between gap-4">
                <div class="min-w-0">
                  <span class="block text-xs font-medium text-foreground">{m.game_computer_use_label({ companion: $companionName })}</span>
                  <p class="mt-0.5 mb-0 text-[11px] leading-relaxed text-muted-foreground">{m.game_computer_use_help()}</p>
                </div>
                <SelectField
                  class="w-32 shrink-0"
                  ariaLabel={m.game_computer_use_label({ companion: $companionName })}
                  testid="profile-computer-use"
                  value={draft.computerUse ?? "ask"}
                  options={COMPUTER_USE_OPTIONS}
                  onchange={(v) => (draft!.computerUse = v as "ask" | "allow" | "deny")}
                />
              </div>
              {/if}
              <div>
                <div class="flex items-center justify-between gap-2">
                  <label for="profile-launch-path" class="block text-xs font-medium text-foreground">{m.game_launch_path()}</label>
                  <Button size="sm" variant="outline" data-testid="profile-detect-games" disabled={detecting} onclick={() => void detectGames()}>
                    {#if detecting}<LoaderCircle class="size-3.5 animate-spin" />{:else}<ScanSearch class="size-3.5" />{/if}{m.game_detect_games()}
                  </Button>
                </div>
                <p class="mt-0.5 mb-0 text-[11px] leading-relaxed text-muted-foreground">{m.game_launch_path_help()}</p>
                <Input id="profile-launch-path" data-testid="profile-launch-path" class="mt-2 font-mono text-xs"
                  placeholder={m.game_launch_path_placeholder()}
                  value={draft.launchPath ?? ""}
                  oninput={(e) => (draft!.launchPath = (e.currentTarget as HTMLInputElement).value)} />
                {#if detected && detected.length}
                  <ul class="mt-2 mb-0 max-h-56 list-none space-y-1 overflow-y-auto p-0" data-testid="profile-detected-games">
                    {#each detected as game (game.id)}
                      <li>
                        <button type="button" data-testid={`profile-detected-${game.id}`} onclick={() => useDetected(game)}
                          class="flex w-full items-center gap-2 rounded-md border border-hairline px-2.5 py-1.5 text-left text-xs transition-colors hover:border-primary/45 hover:bg-primary/6">
                          <span class="min-w-0 flex-1">
                            <span class="block truncate font-medium text-foreground">{game.name}</span>
                            <span class="block truncate text-[11px] text-muted-foreground">{game.installDir}</span>
                          </span>
                          <span class="shrink-0 rounded bg-muted px-1.5 py-0.5 text-[10px] text-muted-foreground">{SOURCE_LABEL[game.source] ?? game.source}</span>
                          {#if game.needsAdmin}<span class="shrink-0 text-[10px] text-warning" title={m.game_needs_admin_title()}>{m.game_needs_admin()}</span>{/if}
                        </button>
                      </li>
                    {/each}
                  </ul>
                {/if}
              </div>
            </div>

            {#if draft.consoleChat}
              <!-- 控制台通道（ADR 032 决定 3）：开关即改游戏目录，不随“保存”走 -->
              <div class="space-y-2 rounded-lg border border-hairline bg-background/40 p-3.5" data-testid="game-profile-console-chat">
                <ToggleField
                  checked={consoleStatus?.enabled ?? false}
                  label={m.game_console_toggle()}
                  desc={m.game_console_toggle_desc({ companion: $companionName })}
                  testid="profile-console-chat"
                  disabled={!consoleStatus || consoleBusy || consoleStatus.gameRunning || !consoleStatus.installDir}
                  onchange={(v) => void toggleConsole(p, v)}
                />
                <p class="m-0 text-[11px] leading-relaxed text-muted-foreground" data-testid="profile-console-chat-status">
                  {#if !consoleStatus}
                    {m.game_console_reading()}
                  {:else if !consoleStatus.installDir}
                    {m.game_console_no_install_dir()}
                  {:else if consoleStatus.gameRunning && !consoleStatus.enabled}
                    {m.game_console_running_enable()}
                  {:else if consoleStatus.gameRunning}
                    {m.game_console_running_disable()}{consoleStatus.needsRestart ? m.game_console_needs_restart() : ""}
                  {:else if consoleStatus.enabled}
                    {m.game_console_enabled({ time: consoleStatus.writtenAt ?? m.game_console_time_unknown(), key: consoleStatus.triggerKey })}{consoleStatus.keyMismatch ? m.game_console_key_mismatch() : ""}{m.game_console_cfg_dir({ dir: consoleStatus.cfgDir ?? "" })}
                  {:else}
                    {m.game_console_cfg_dir({ dir: consoleStatus.cfgDir ?? "" })}
                  {/if}
                </p>
              </div>
            {/if}

            <div>
              <button
                class="text-xs text-muted-foreground underline-offset-2 transition-colors hover:text-foreground hover:underline"
                onclick={() => (showTechnical = !showTechnical)}
              >
                {showTechnical ? m.game_advanced_collapse() : m.game_advanced()}
              </button>
              {#if showTechnical}
                <div class="mt-3 space-y-4 rounded-lg border border-hairline bg-background/40 p-3.5">
                  <div>
                    {@render field(m.game_chat_keys(), m.game_chat_keys_hint())}
                    <div class="mt-2 grid grid-cols-1 gap-2 sm:grid-cols-2">
                      {#each [{ key: "openChatKey", label: m.game_open_chat_key(), empty: m.game_open_chat_key_empty(), clear: m.game_open_chat_key_clear() }, { key: "sendKey", label: m.game_send_key(), empty: m.game_send_key_empty(), clear: m.game_send_key_clear() }] as const as item}
                        <div>
                          <span class="text-[11px] text-muted-foreground">{item.label}</span>
                          <div class="mt-1 flex gap-1.5">
                            <button
                              class="h-8 min-w-0 flex-1 truncate rounded-lg border border-input bg-background/60 px-2.5 text-left text-xs text-foreground transition-colors hover:bg-muted focus-visible:border-ring focus-visible:outline-none disabled:opacity-50"
                              disabled={capturingKey !== null}
                              onclick={() => void captureProfileKey(item.key)}
                            >
                              {capturingKey === item.key ? m.game_press_key() : draft[item.key] || item.empty}
                            </button>
                            <Button size="sm" variant="ghost" disabled={capturingKey !== null || !draft[item.key]} title={item.clear}
                              onclick={() => clearProfileKey(item.key)}>{m.game_clear()}</Button>
                          </div>
                        </div>
                      {/each}
                    </div>
                  </div>
                  <ToggleField
                    checked={draft.preferClipboardPaste}
                    label={m.game_clipboard_paste()}
                    desc={m.game_clipboard_paste_desc()}
                    onchange={(v) => (draft!.preferClipboardPaste = v)}
                  />
                  <div class="space-y-3">
                    {@render field(m.game_send_timing(), m.game_send_timing_hint())}
                    {@render slider(m.game_pre_open_delay(), "preOpenDelayMs")}
                    {@render slider(m.game_pre_paste_delay(), "prePasteDelayMs")}
                    {@render slider(m.game_pre_send_delay(), "preSendDelayMs")}
                  </div>
                </div>
              {/if}
            </div>

            <div class="flex items-center justify-end gap-2">
              <Button size="sm" variant="outline" onclick={closeEditor}>{m.game_cancel()}</Button>
              <Button size="sm" disabled={saving} onclick={() => void onSave()}>{saving ? m.game_saving() : m.game_save()}</Button>
            </div>
          </div>
        {/if}
      </div>
    {/each}
  </SettingsSection>

  <!-- dev:web 导入热词用隐藏文件选择器 -->
  <input
    bind:this={fileInput}
    type="file"
    accept=".txt,text/plain"
    class="hidden"
    onchange={(e) => void onImportFile(e)}
  />
  <!-- dev:web 导入 .zip 配置包用隐藏文件选择器 -->
  <input
    bind:this={pkgFileInput}
    type="file"
    accept=".zip,application/zip"
    class="hidden"
    onchange={(e) => void onImportPkgFile(e)}
  />
</SettingsPage>
