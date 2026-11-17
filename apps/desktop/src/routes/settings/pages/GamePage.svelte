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
   *   保存时整份 profile 原样带回，新字段不会丢。
   * - 版式：SettingsPage / SettingsSection（参照 DeepSeek Harness 设置页）。
   */
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
    isTauri,
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

  /** 让琴音操作（game.act）的三档。 */
  const COMPUTER_USE_OPTIONS = [
    { value: "ask", label: "先问我", description: "每段对话第一次操作前问一次" },
    { value: "allow", label: "一直允许", description: "不再询问" },
    { value: "deny", label: "不允许", description: "琴音不在这个游戏里动鼠标键盘" },
  ];

  let profiles = $state<GameProfile[]>([]);
  /** 启动路径的“自动检测”结果（null = 还没检测）。 */
  let detected = $state<InstalledGame[] | null>(null);
  let detecting = $state(false);
  const SOURCE_LABEL: Record<string, string> = {
    steam: "Steam", tencent: "腾讯", riot: "Riot", hoyoplay: "米哈游", epic: "Epic", wegame: "WeGame", uninstall: "系统已安装程序",
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
      if (!detected.length) toast(false, "没有检测到已安装的游戏，可以手动填写启动路径");
    } catch (error) {
      toast(false, `检测失败：${errText(error)}`);
    } finally {
      detecting = false;
    }
  }

  function useDetected(game: InstalledGame) {
    if (!draft || !game.launch) return;
    draft.launchPath = game.launch;
    detected = null;
    toast(true, `已填入「${game.name}」的启动方式，记得保存`);
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

  /** 内置 profile（删除语义 = 恢复出厂）；当前内置集仍为 lol / generic */
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
      toast(false, `加载游戏配置失败：${errText(e)}`);
    }
  });

  async function onActivate(id: string) {
    try {
      settingsStore.set(await updateSettings({ activeProfileId: id }));
      toast(true, `已切换游戏配置：${profiles.find((p) => p.id === id)?.displayName ?? id}`);
    } catch (e) {
      toast(false, `保存失败：${errText(e)}`);
    }
  }

  // ---------- 编辑器 ----------

  function openEditor(p: GameProfile) {
    editingId = p.id;
    draft = $state.snapshot(p);
    hotwordsText = p.hotwords.join("\n");
    showTechnical = false;
  }

  function closeEditor() {
    captureCleanup?.();
    captureCleanup = null;
    capturingKey = null;
    editingId = null;
    draft = null;
    hotwordsText = "";
    showTechnical = false;
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
        toastWarn("按键录入超时，请重试");
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
          ? `配置已保存，已合并 ${duplicates} 个重复热词（下次识别生效）`
          : "游戏配置已保存（热词下次识别生效）",
      );
      closeEditor();
    } catch (e) {
      toast(false, `保存失败：${errText(e)}`);
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
          filters: [{ name: "热词表", extensions: ["txt"] }],
        });
        if (!path) return; // 用户取消，不打扰
        const n = await exportHotwords(p.id, path);
        toast(true, `已导出 ${n} 个热词 → ${path}`);
      } else {
        const blob = new Blob([formatExport(p.hotwords)], { type: "text/plain;charset=utf-8" });
        const url = URL.createObjectURL(blob);
        const a = document.createElement("a");
        a.href = url;
        a.download = `${p.id}-hotwords.txt`;
        a.click();
        URL.revokeObjectURL(url);
        toast(true, `已导出 ${p.hotwords.length} 个热词`);
      }
    } catch (e) {
      toast(false, `导出失败：${errText(e)}`);
    }
  }

  async function onImport(p: GameProfile) {
    if (importing) return;
    if (isTauri) {
      try {
        const sel = await openDialog({
          multiple: false,
          filters: [{ name: "热词表", extensions: ["txt"] }],
        });
        if (!sel || typeof sel !== "string") return; // 用户取消
        importing = true;
        const report = await importHotwords(p.id, sel);
        await refreshAfterImport(p.id);
        toast(true, `导入完成：新增 ${report.added} 条，跳过 ${report.duplicates} 条重复`);
      } catch (e) {
        toast(false, `导入失败：${errText(e)}`);
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
      toast(true, `导入完成：新增 ${added} 条，跳过 ${duplicates} 条重复`);
    } catch (err) {
      toast(false, `导入失败：${errText(err)}`);
    } finally {
      importing = false;
    }
  }

  // ---------- 整包导入导出（.zip：profile.json + icon） ----------

  /** 导出整包：含 icon，供分享/编辑成别的游戏再导入 */
  async function onExportPackage(p: GameProfile) {
    if (!isTauri) {
      toastWarn("dev:web 环境不支持 .zip 导出，请在 Tauri 应用中验证");
      return;
    }
    try {
      const path = await saveDialog({
        defaultPath: `${p.displayName}.zip`,
        filters: [{ name: "Kotone 配置包", extensions: ["zip"] }],
      });
      if (!path) return; // 用户取消
      await exportProfile(p.id, path);
      toast(true, `已导出配置包 → ${path}`);
    } catch (e) {
      toast(false, `导出失败：${errText(e)}`);
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
        filters: [{ name: "Kotone 配置包", extensions: ["zip"] }],
      });
      if (!sel || typeof sel !== "string") return; // 用户取消
      importingPackage = true;
      const imported = await importProfile(sel);
      await loadProfiles();
      toast(true, `已导入「${imported.displayName}」`);
    } catch (e) {
      toast(false, `导入失败：${errText(e)}`);
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
      toast(true, `已导入「${imported.displayName}」（dev:web 模拟）`);
    } catch (err) {
      toast(false, `导入失败：${errText(err)}`);
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
        kind === "reset" ? `「${p.displayName}」已恢复默认配置` : `已删除「${p.displayName}」`,
      );
    } catch (e) {
      toast(false, `操作失败：${errText(e)}`);
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

<SettingsPage title="游戏适配" description="每个游戏一套打法：聊天键、词表、节奏，以及琴音能不能替你在这个游戏里按键。">
  {#snippet actions()}
    <Button variant="outline" size="sm" disabled={importingPackage} onclick={() => void onImportPackage()}>
      <Upload class="size-3.5" />{importingPackage ? "导入中…" : "导入配置包"}
    </Button>
  {/snippet}

  <SettingsSection title="游戏配置" description="把分享来的 .zip 配置包导进来，或导出自己的去分享。" flush>
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
                <span class="shrink-0 rounded-full bg-primary/12 px-2 py-0.5 text-[10px] font-medium text-primary">激活中</span>
              {/if}
            </p>
            <p class="mt-0.5 mb-0 truncate text-xs text-muted-foreground">
              {p.processNames.length > 0 ? `适配 ${p.processNames.length} 个进程` : "适用于所有窗口"}
              · 热词 {p.hotwords.length} 个{p.allowAutomationInput === false ? " · 不允许自动化输入" : ""}{p.computerUse === "allow" ? " · 琴音可直接操作" : p.computerUse === "deny" ? " · 不让琴音操作" : ""}
            </p>
          </div>
          <div class="flex shrink-0 flex-wrap items-center gap-1.5">
            <Button size="sm" variant={editing ? "secondary" : "outline"} onclick={() => (editing ? closeEditor() : openEditor(p))}>
              {editing ? "收起" : "编辑"}
            </Button>
            <Button size="sm" variant="ghost" title="导出 .zip 配置包（可编辑成别的游戏后重新导入）" onclick={() => void onExportPackage(p)}>
              导出
            </Button>
            {#if !active}
              <Button size="sm" variant="outline" onclick={() => void onActivate(p.id)}>激活</Button>
            {/if}
            <Button size="sm" variant={deletingId === p.id ? "destructive" : "ghost"}
              class={deletingId === p.id ? "" : "text-muted-foreground"}
              title={isBuiltin(p) ? "恢复默认配置（还原内置热词与图标）" : "删除该配置"}
              onclick={() => void onDeleteProfile(p)}>
              {deletingId === p.id
                ? isBuiltin(p)
                  ? "再点确认重置"
                  : "再点确认删除"
                : isBuiltin(p)
                  ? "重置"
                  : "删除"}
            </Button>
          </div>
        </div>

        {#if p.channels && p.channels.length > 1}
          <!-- 多频道声明（ADR-008）：写清当前适配支持哪些频道、按什么键切换 -->
          <div class="mt-3 flex flex-wrap items-center gap-1.5 pl-13">
            <span class="text-[11px] text-muted-foreground">聊天频道</span>
            {#each p.channels as ch}
              <span
                class="rounded-full px-2 py-0.5 text-[11px] {ch.default ? 'bg-primary/12 text-primary' : 'bg-muted text-foreground/75'}"
                title={ch.textPrefix
                  ? `发送前缀 ${ch.textPrefix}`
                  : ch.openChatKey
                    ? `开聊天框按 ${ch.openChatKey}`
                    : "沿用默认开聊天框按键"}
              >
                {ch.displayName}{ch.default ? "（默认）" : ""}
              </span>
            {/each}
            <span class="text-[11px] text-muted-foreground">
              · 按 {$settingsStore?.channelCycleHotkey ?? "Shift+CapsLock"} 循环切换（「高级」页可改）
            </span>
          </div>
        {/if}

        {#if editing && draft}
          <!-- 普通用户只编辑热词；聊天键和注入策略收进高级输入设置。 -->
          <div class="mt-4 space-y-5 rounded-xl border border-hairline bg-muted/30 p-4" data-testid="game-profile-editor">
            <div>
              <div class="flex items-center justify-between gap-2">
                {@render field(`自定义词汇（${hotwordCount} 个，下次识别生效）`)}
                <div class="flex shrink-0 gap-1.5">
                  <Button size="xs" variant="outline" disabled={importing} onclick={() => void onExport(p)}><Download class="size-3" />导出</Button>
                  <Button size="xs" variant="outline" disabled={importing} onclick={() => void onImport(p)}><Upload class="size-3" />{importing ? "导入中…" : "导入"}</Button>
                </div>
              </div>
              <!-- 批量文本编辑：每行一个词条，与导入/导出 txt 格式同构（几百上千条可用） -->
              <textarea
                bind:value={hotwordsText}
                rows={10}
                placeholder={"每行一个词条，保存时自动去重\n例：\n打野\n中路MISS\n纳什男爵"}
                spellcheck="false"
                class="mt-2 w-full resize-y rounded-lg border border-input bg-background/60 px-3 py-2 font-mono text-xs leading-relaxed text-foreground outline-none placeholder:text-muted-foreground focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/40"
              ></textarea>
              <p class="mt-1.5 mb-0 text-[11px] text-muted-foreground">
                每行一个词条，保存时自动去重；导入为合并模式（UTF-8 文本同格式，跳过空行与重复，导入立即保存）。
              </p>
            </div>

            <div>
              {@render field("陪玩说明（可选）")}
              <textarea
                bind:value={draft.companionNotes}
                rows={3}
                maxlength={500}
                data-testid="profile-companion-notes"
                placeholder="写给琴音的这个游戏的说明，例如：聊天在左下角，[队伍] 前缀是队友，[全部] 是所有人"
                class="mt-2 w-full resize-y rounded-lg border border-input bg-background/60 px-3 py-2 text-xs leading-relaxed text-foreground outline-none placeholder:text-muted-foreground focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/40"
              ></textarea>
              <p class="mt-1.5 mb-0 text-[11px] text-muted-foreground">
                陪玩时会把这段说明连同聊天截图的文字一起交给 AI；游戏适配写在这里，不需要为每个游戏写规则。{draft.chatRegion ? "已记住聊天框位置，可在「陪玩设置」里改回整窗识别。" : ""}
              </p>
            </div>

            <!-- 琴音与自动化（游戏配置的新字段；保存时整份 profile 原样带回） -->
            <div class="space-y-4 rounded-lg border border-hairline bg-background/40 p-3.5" data-testid="game-profile-agent">
              <ToggleField
                checked={draft.allowAutomationInput ?? true}
                label="允许自动化输入"
                desc="关掉后，自动化程序不会对这个游戏按键、点鼠标。"
                testid="profile-allow-automation-input"
                onchange={(v) => (draft!.allowAutomationInput = v)}
              />
              <div class="flex items-start justify-between gap-4">
                <div class="min-w-0">
                  <span class="block text-xs font-medium text-foreground">让琴音操作</span>
                  <p class="mt-0.5 mb-0 text-[11px] leading-relaxed text-muted-foreground">琴音一边看画面一边点鼠标、按键（例如帮你找背包里的东西），右下角会提示“正在操作 · 按 Esc 停止”。操作时按 Esc 就停（这个 Esc 不会传给游戏）；Ctrl+Shift+F12 或说“停下”也能停。有反作弊的游戏能不能用模拟点击，请你自己判断。</p>
                </div>
                <SelectField
                  class="w-32 shrink-0"
                  ariaLabel="让琴音操作"
                  testid="profile-computer-use"
                  value={draft.computerUse ?? "ask"}
                  options={COMPUTER_USE_OPTIONS}
                  onchange={(v) => (draft!.computerUse = v as "ask" | "allow" | "deny")}
                />
              </div>
              <div>
                <div class="flex items-center justify-between gap-2">
                  <label for="profile-launch-path" class="block text-xs font-medium text-foreground">启动路径</label>
                  <Button size="sm" variant="outline" data-testid="profile-detect-games" disabled={detecting} onclick={() => void detectGames()}>
                    {#if detecting}<LoaderCircle class="size-3.5 animate-spin" />{:else}<ScanSearch class="size-3.5" />{/if}自动检测
                  </Button>
                </div>
                <p class="mt-0.5 mb-0 text-[11px] leading-relaxed text-muted-foreground">程序、快捷方式或 steam:// 之类的链接。点“自动检测”可以从本机装的游戏里选（Steam、英雄联盟、Riot、米哈游、Epic、WeGame）；留空时琴音也会自动找。</p>
                <Input id="profile-launch-path" data-testid="profile-launch-path" class="mt-2 font-mono text-xs"
                  placeholder="例如：C:\Games\Game.exe 或 steam://rungameid/570"
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
                          {#if game.needsAdmin}<span class="shrink-0 text-[10px] text-warning" title="启动时 Windows 会请你授权">需授权</span>{/if}
                        </button>
                      </li>
                    {/each}
                  </ul>
                {/if}
              </div>
            </div>

            <div>
              <button
                class="text-xs text-muted-foreground underline-offset-2 transition-colors hover:text-foreground hover:underline"
                onclick={() => (showTechnical = !showTechnical)}
              >
                {showTechnical ? "收起高级输入设置" : "高级输入设置"}
              </button>
              {#if showTechnical}
                <div class="mt-3 space-y-4 rounded-lg border border-hairline bg-background/40 p-3.5">
                  <div>
                    {@render field("聊天键", "点击按键名称后直接录入键盘组合或鼠标侧键；清空表示跳过该阶段。")}
                    <div class="mt-2 grid grid-cols-1 gap-2 sm:grid-cols-2">
                      {#each [{ key: "openChatKey", label: "打开聊天框", empty: "不打开聊天框", clear: "清空：发送时不按打开聊天框键" }, { key: "sendKey", label: "发送消息", empty: "不发送消息", clear: "清空：发送时不按发送键" }] as const as item}
                        <div>
                          <span class="text-[11px] text-muted-foreground">{item.label}</span>
                          <div class="mt-1 flex gap-1.5">
                            <button
                              class="h-8 min-w-0 flex-1 truncate rounded-lg border border-input bg-background/60 px-2.5 text-left text-xs text-foreground transition-colors hover:bg-muted focus-visible:border-ring focus-visible:outline-none disabled:opacity-50"
                              disabled={capturingKey !== null}
                              onclick={() => void captureProfileKey(item.key)}
                            >
                              {capturingKey === item.key ? "请按下按键…" : draft[item.key] || item.empty}
                            </button>
                            <Button size="sm" variant="ghost" disabled={capturingKey !== null || !draft[item.key]} title={item.clear}
                              onclick={() => clearProfileKey(item.key)}>清空</Button>
                          </div>
                        </div>
                      {/each}
                    </div>
                  </div>
                  <ToggleField
                    checked={draft.preferClipboardPaste}
                    label="使用剪贴板粘贴"
                    desc="仅在目标游戏无法正常接收逐字输入时开启"
                    onchange={(v) => (draft!.preferClipboardPaste = v)}
                  />
                  <div class="space-y-3">
                    {@render field("发送时序", "聊天框还没打开就开始打字时，把「打开聊天后等待」调到 50–100ms。")}
                    {@render slider("打开聊天后等待", "preOpenDelayMs")}
                    {@render slider("粘贴前等待", "prePasteDelayMs")}
                    {@render slider("发送前等待", "preSendDelayMs")}
                  </div>
                </div>
              {/if}
            </div>

            <div class="flex items-center justify-end gap-2">
              <Button size="sm" variant="outline" onclick={closeEditor}>取消</Button>
              <Button size="sm" disabled={saving} onclick={() => void onSave()}>{saving ? "保存中…" : "保存"}</Button>
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
