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
   */
  import { onMount } from "svelte";
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
    isTauri,
    type GameProfile,
  } from "../../../lib/ipc";
  import { settingsStore, toast, toastWarn, errText } from "../../../lib/stores/ui";
  import { spotlight } from "../../../lib/actions/spotlight";
  import Toggle from "../../../lib/components/Toggle.svelte";

  let profiles = $state<GameProfile[]>([]);

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
    editingId = null;
    draft = null;
    hotwordsText = "";
    showTechnical = false;
  }

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

<div class="px-6 py-5">
  <h1 class="text-lg font-bold">
    <span class="kotone-gradient-text">游戏适配</span>
  </h1>
  <p class="mt-0.5 text-[11px] text-white/45">每个游戏一套打法：聊天键、词表、节奏</p>

  <div class="mt-3 flex items-center justify-between gap-3">
    <p class="min-w-0 text-[11px] text-white/40">
      把分享来的 .zip 配置包导进来，或导出自己的去分享
    </p>
    <button
      class="shrink-0 rounded-lg bg-white/10 px-3 py-1.5 text-xs font-semibold text-white/80 transition hover:bg-white/20 active:scale-95 disabled:opacity-50"
      disabled={importingPackage}
      onclick={() => void onImportPackage()}
    >
      {importingPackage ? "导入中…" : "导入配置包"}
    </button>
  </div>

  <div class="mt-3 flex flex-col gap-3">
    {#each profiles as p}
      {@const active = $settingsStore?.activeProfileId === p.id}
      {@const editing = editingId === p.id && draft !== null}
      <div
        use:spotlight
        class="kotone-card kotone-spotlight p-4 {active ? 'border-kotone-cyan/50 shadow-glow-cyan' : ''}"
      >
        <div class="flex items-center gap-3">
          <!-- 游戏图标：有 icon 用 profile 图标（IPC 加载 blob URL），否则用占位 -->
          <span
            class="flex h-11 w-11 shrink-0 items-center justify-center overflow-hidden rounded-xl
              {active ? 'bg-kotone-cyan/15' : 'bg-white/8'}"
          >
            {#if p.icon && iconUrls[p.id]}
              <img src={iconUrls[p.id]} alt={p.displayName} class="h-full w-full object-contain" />
            {:else if p.processNames.length === 0}
              <!-- lucide: globe -->
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="h-5 w-5 text-white/60"><circle cx="12" cy="12" r="10"/><path d="M12 2a14.5 14.5 0 0 0 0 20 14.5 14.5 0 0 0 0-20"/><path d="M2 12h20"/></svg>
            {:else}
              <!-- lucide: gamepad-2 -->
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="h-5 w-5 text-white/60"><line x1="6" x2="10" y1="11" y2="11"/><line x1="8" x2="8" y1="9" y2="13"/><line x1="15" x2="15.01" y1="12" y2="12"/><line x1="18" x2="18.01" y1="10" y2="10"/><path d="M17.32 5H6.68a4 4 0 0 0-3.978 3.59c-.006.052-.01.101-.017.152C2.604 9.416 2 14.456 2 16a3 3 0 0 0 3 3c1 0 1.5-.5 2-1l1.414-1.414A2 2 0 0 1 9.828 16h4.344a2 2 0 0 1 1.414.586L17 18c.5.5 1 1 2 1a3 3 0 0 0 3-3c0-1.545-.604-6.584-.685-7.258-.007-.05-.011-.1-.017-.151A4 4 0 0 0 17.32 5z"/></svg>
            {/if}
          </span>
          <div class="min-w-0 flex-1">
            <p class="flex items-center gap-2 text-sm font-semibold">
              <span class="truncate">{p.displayName}</span>
              {#if active}
                <span class="shrink-0 rounded bg-kotone-cyan/20 px-1.5 py-0.5 text-[10px] text-kotone-cyan">激活中</span>
              {/if}
            </p>
            <p class="mt-0.5 truncate text-[11px] text-white/45">
              {p.processNames.length > 0 ? `适配 ${p.processNames.length} 个进程` : "适用于所有窗口"}
              · 热词 {p.hotwords.length} 个
            </p>
          </div>
          <button
            class="shrink-0 rounded-lg px-3 py-1.5 text-xs font-semibold ring-1 transition active:scale-95 {editing
              ? 'bg-kotone-cyan/15 text-kotone-cyan ring-kotone-cyan/40'
              : 'bg-white/10 text-white/80 ring-white/10 hover:bg-white/20'}"
            onclick={() => (editing ? closeEditor() : openEditor(p))}
          >
            {editing ? "收起" : "编辑"}
          </button>
          <button
            class="shrink-0 rounded-lg bg-white/8 px-2.5 py-1.5 text-[11px] text-white/55 ring-1 ring-white/12 transition hover:bg-white/15 hover:text-white/85 active:scale-95"
            title="导出 .zip 配置包（可编辑成别的游戏后重新导入）"
            onclick={() => void onExportPackage(p)}
          >
            导出
          </button>
          {#if !active}
            <button
              class="shrink-0 rounded-lg bg-white/10 px-3 py-1.5 text-xs font-semibold text-white/80 transition hover:bg-white/20 active:scale-95"
              onclick={() => void onActivate(p.id)}
            >
              激活
            </button>
          {/if}
          <button
            class="shrink-0 rounded-lg px-2.5 py-1.5 text-[11px] ring-1 transition active:scale-95 {deletingId ===
            p.id
              ? 'bg-kotone-pink/20 text-kotone-pink ring-kotone-pink/50'
              : 'bg-white/8 text-white/55 ring-white/12 hover:bg-white/15 hover:text-white/85'}"
            title={isBuiltin(p) ? "恢复默认配置（还原内置热词与图标）" : "删除该配置"}
            onclick={() => void onDeleteProfile(p)}
          >
            {deletingId === p.id
              ? isBuiltin(p)
                ? "再点确认重置"
                : "再点确认删除"
              : isBuiltin(p)
                ? "重置"
                : "删除"}
          </button>
        </div>

        {#if p.channels && p.channels.length > 1}
          <!-- 多频道声明（ADR-008）：写清当前适配支持哪些频道、按什么键切换 -->
          <div class="mt-3 flex flex-wrap items-center gap-1.5 border-t border-white/8 pt-3">
            <span class="text-[10px] text-white/40">聊天频道</span>
            {#each p.channels as ch}
              <span
                class="rounded px-1.5 py-0.5 text-[10px] {ch.default
                  ? 'bg-kotone-cyan/15 text-kotone-cyan'
                  : 'bg-white/8 text-white/60'}"
                title={ch.textPrefix
                  ? `发送前缀 ${ch.textPrefix}`
                  : ch.openChatKey
                    ? `开聊天框按 ${ch.openChatKey}`
                    : "沿用默认开聊天框按键"}
              >
                {ch.displayName}{ch.default ? "（默认）" : ""}
              </span>
            {/each}
            <span class="text-[10px] text-white/35">
              · 按 {$settingsStore?.channelCycleHotkey ?? "Shift+CapsLock"} 循环切换（「高级」页可改）
            </span>
          </div>
        {/if}

        {#if editing && draft}
          <!-- 普通用户只编辑热词；聊天键和注入策略收进高级输入设置。 -->
          <div class="mt-4 border-t border-white/8 pt-4">
            <div>
              <div class="flex items-center justify-between gap-2">
                <p class="text-[10px] font-semibold tracking-wide text-white/40">
                  自定义词汇（{hotwordCount} 个，下次识别生效）
                </p>
                <div class="flex shrink-0 gap-1.5">
                  <button
                    class="rounded-lg bg-white/8 px-2.5 py-1 text-[11px] text-white/55 ring-1 ring-white/12 transition hover:bg-white/15 hover:text-white/85 active:scale-95 disabled:opacity-50"
                    disabled={importing}
                    onclick={() => void onExport(p)}
                  >
                    导出
                  </button>
                  <button
                    class="rounded-lg bg-white/8 px-2.5 py-1 text-[11px] text-white/55 ring-1 ring-white/12 transition hover:bg-white/15 hover:text-white/85 active:scale-95 disabled:opacity-50"
                    disabled={importing}
                    onclick={() => void onImport(p)}
                  >
                    {importing ? "导入中…" : "导入"}
                  </button>
                </div>
              </div>
              <!-- 批量文本编辑：每行一个词条，与导入/导出 txt 格式同构（几百上千条可用） -->
              <textarea
                bind:value={hotwordsText}
                rows={10}
                placeholder={"每行一个词条，保存时自动去重\n例：\n打野\n中路MISS\n纳什男爵"}
                spellcheck="false"
                class="kotone-scroll mt-2 w-full resize-y rounded-lg bg-white/8 px-2.5 py-2 font-mono text-xs leading-relaxed ring-1 ring-white/15 outline-none placeholder:text-white/30 focus:ring-kotone-cyan/60"
              ></textarea>
              <p class="mt-1.5 text-[10px] text-white/35">
                每行一个词条，保存时自动去重；导入为合并模式（UTF-8 文本同格式，跳过空行与重复，导入立即保存）。
              </p>
            </div>

            <button
              class="mt-4 text-[11px] text-white/45 underline-offset-2 transition hover:text-white/75 hover:underline"
              onclick={() => (showTechnical = !showTechnical)}
            >
              {showTechnical ? "收起高级输入设置" : "高级输入设置"}
            </button>
            {#if showTechnical}
              <div class="mt-3 rounded-lg bg-white/4 p-3 ring-1 ring-white/8">
                <p class="text-[10px] font-semibold tracking-wide text-white/40">聊天键</p>
                <div class="mt-2 grid grid-cols-2 gap-2">
                  <label class="block">
                    <span class="text-[11px] text-white/55">打开聊天框</span>
                    <input
                      bind:value={draft.openChatKey}
                      placeholder="如 Enter"
                      spellcheck="false"
                      class="mt-1 w-full rounded-lg bg-white/8 px-2.5 py-1.5 text-xs ring-1 ring-white/15 outline-none placeholder:text-white/30 focus:ring-kotone-cyan/60"
                    />
                  </label>
                  <label class="block">
                    <span class="text-[11px] text-white/55">发送消息</span>
                    <input
                      bind:value={draft.sendKey}
                      placeholder="如 Enter"
                      spellcheck="false"
                      class="mt-1 w-full rounded-lg bg-white/8 px-2.5 py-1.5 text-xs ring-1 ring-white/15 outline-none placeholder:text-white/30 focus:ring-kotone-cyan/60"
                    />
                  </label>
                </div>
                <div class="mt-4">
                  <Toggle
                    checked={draft.preferClipboardPaste}
                    label="使用剪贴板粘贴"
                    desc="仅在目标游戏无法正常接收逐字输入时开启"
                    onchange={(v) => (draft!.preferClipboardPaste = v)}
                  />
                </div>
                <p class="mt-4 text-[10px] font-semibold tracking-wide text-white/40">发送时序</p>
                <p class="mt-1 text-[10px] leading-relaxed text-white/35">
                  聊天框还没打开就开始打字时，把「打开聊天后等待」调到 50–100ms。
                </p>
                <label class="mt-3 block">
                  <div class="flex items-center justify-between">
                    <span class="text-[11px] text-white/55">打开聊天后等待</span>
                    <span class="text-[11px] text-white/45">{draft.preOpenDelayMs} ms</span>
                  </div>
                  <input
                    type="range"
                    min="0"
                    max="300"
                    step="10"
                    bind:value={draft.preOpenDelayMs}
                    class="mt-1.5 w-full accent-kotone-cyan"
                  />
                </label>
                <label class="mt-3 block">
                  <div class="flex items-center justify-between">
                    <span class="text-[11px] text-white/55">粘贴前等待</span>
                    <span class="text-[11px] text-white/45">{draft.prePasteDelayMs} ms</span>
                  </div>
                  <input
                    type="range"
                    min="0"
                    max="300"
                    step="10"
                    bind:value={draft.prePasteDelayMs}
                    class="mt-1.5 w-full accent-kotone-cyan"
                  />
                </label>
                <label class="mt-3 block">
                  <div class="flex items-center justify-between">
                    <span class="text-[11px] text-white/55">发送前等待</span>
                    <span class="text-[11px] text-white/45">{draft.preSendDelayMs} ms</span>
                  </div>
                  <input
                    type="range"
                    min="0"
                    max="300"
                    step="10"
                    bind:value={draft.preSendDelayMs}
                    class="mt-1.5 w-full accent-kotone-cyan"
                  />
                </label>
              </div>
            {/if}

            <div class="mt-4 flex items-center justify-end gap-2">
              <button
                class="rounded-lg bg-white/10 px-3 py-1.5 text-xs text-white/70 transition hover:bg-white/20 active:scale-95"
                onclick={closeEditor}
              >
                取消
              </button>
              <button
                class="rounded-lg bg-kotone-cyan px-3 py-1.5 text-xs font-semibold text-kotone-deep transition hover:brightness-110 active:scale-95 disabled:opacity-50"
                disabled={saving}
                onclick={() => void onSave()}
              >
                {saving ? "保存中…" : "保存"}
              </button>
            </div>
          </div>
        {/if}
      </div>
    {/each}
  </div>

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
</div>
