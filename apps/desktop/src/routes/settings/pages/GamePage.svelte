<script lang="ts">
  /*
   * 游戏适配页（瘦身版）：profile 游戏卡片（激活开关）+ 内联 profile 编辑器。
   * - 编辑器：发送键配置（openChatKey/sendKey）、聊天框模式（preferClipboardPaste）、
   *   热词管理（chips 逐条删除 / 添加 / 导出 / 导入）；
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
  /** 编辑草稿（structuredClone 深拷贝，保存才落盘） */
  let draft = $state<GameProfile | null>(null);
  let newHotword = $state("");
  let saving = $state(false);
  let importing = $state(false);
  /** dev:web 导入用隐藏文件选择器 */
  let fileInput = $state<HTMLInputElement | null>(null);
  let importTargetId = $state<string | null>(null);

  onMount(async () => {
    try {
      profiles = await listProfiles();
    } catch (e) {
      toast(false, `加载 profile 失败：${errText(e)}`);
    }
  });

  async function onActivate(id: string) {
    try {
      settingsStore.set(await updateSettings({ activeProfileId: id }));
      toast(true, `已激活 profile：${profiles.find((p) => p.id === id)?.displayName ?? id}`);
    } catch (e) {
      toast(false, `保存失败：${errText(e)}`);
    }
  }

  // ---------- 编辑器 ----------

  function openEditor(p: GameProfile) {
    editingId = p.id;
    draft = structuredClone(p);
    newHotword = "";
  }

  function closeEditor() {
    editingId = null;
    draft = null;
    newHotword = "";
  }

  async function onSave() {
    if (!draft || saving) return;
    saving = true;
    try {
      await saveProfile(draft);
      profiles = profiles.map((p) => (p.id === draft!.id ? structuredClone(draft!) : p));
      toast(true, "profile 已保存（热词下次识别生效）");
      closeEditor();
    } catch (e) {
      toast(false, `保存失败：${errText(e)}`);
    } finally {
      saving = false;
    }
  }

  function addHotword() {
    if (!draft) return;
    const w = newHotword.trim();
    if (!w) return;
    if (draft.hotwords.includes(w)) {
      toastWarn(`热词已存在：${w}`);
      return;
    }
    draft.hotwords = [...draft.hotwords, w];
    newHotword = "";
  }

  function removeHotword(w: string) {
    if (!draft) return;
    draft.hotwords = draft.hotwords.filter((x) => x !== w);
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

  /** 导入后刷新 profile 列表；若编辑器开着同一 profile，同步草稿里的热词 */
  async function refreshAfterImport(profileId: string) {
    profiles = await listProfiles();
    if (editingId === profileId && draft) {
      const fresh = profiles.find((p) => p.id === profileId);
      if (fresh) draft.hotwords = [...fresh.hotwords];
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
</script>

<div class="px-6 py-5">
  <h1 class="text-lg font-bold">
    <span class="kotone-gradient-text">游戏适配</span>
  </h1>
  <p class="mt-0.5 text-[11px] text-white/45">每个游戏一套打法：聊天键、词表、节奏</p>

  <div class="mt-4 flex flex-col gap-3">
    {#each profiles as p}
      {@const active = $settingsStore?.activeProfileId === p.id}
      {@const editing = editingId === p.id && draft !== null}
      <div
        use:spotlight
        class="kotone-card kotone-spotlight p-4 {active ? 'border-kotone-cyan/50 shadow-glow-cyan' : ''}"
      >
        <div class="flex items-center gap-3">
          <!-- 图标占位 -->
          <span
            class="flex h-11 w-11 shrink-0 items-center justify-center rounded-xl text-lg
              {active ? 'bg-kotone-cyan/15' : 'bg-white/8'}"
          >
            {p.processNames.length === 0 ? "🌐" : "🎮"}
          </span>
          <div class="min-w-0 flex-1">
            <p class="flex items-center gap-2 text-sm font-semibold">
              <span class="truncate">{p.displayName}</span>
              {#if active}
                <span class="shrink-0 rounded bg-kotone-cyan/20 px-1.5 py-0.5 text-[10px] text-kotone-cyan">激活中</span>
              {/if}
            </p>
            <p class="mt-0.5 truncate text-[11px] text-white/45">
              {p.processNames.length === 0 ? "通配任意前台窗口" : p.processNames.join(" / ")}
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
          {#if !active}
            <button
              class="shrink-0 rounded-lg bg-white/10 px-3 py-1.5 text-xs font-semibold text-white/80 transition hover:bg-white/20 active:scale-95"
              onclick={() => void onActivate(p.id)}
            >
              激活
            </button>
          {/if}
        </div>

        {#if editing && draft}
          <!-- 内联编辑器：发送键配置 + 聊天框模式 + 热词管理 -->
          <div class="mt-4 border-t border-white/8 pt-4">
            <p class="text-[10px] font-semibold tracking-wide text-white/40">发送键配置</p>
            <div class="mt-2 grid grid-cols-2 gap-2">
              <label class="block">
                <span class="text-[11px] text-white/55">打开聊天框键</span>
                <input
                  bind:value={draft.openChatKey}
                  placeholder="如 Enter"
                  spellcheck="false"
                  class="mt-1 w-full rounded-lg bg-white/8 px-2.5 py-1.5 text-xs ring-1 ring-white/15 outline-none placeholder:text-white/30 focus:ring-kotone-cyan/60"
                />
              </label>
              <label class="block">
                <span class="text-[11px] text-white/55">发送键</span>
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
                label="剪贴板粘贴模式"
                desc="开启：模拟 Ctrl+V 粘贴（快、长文本稳定）；关闭：Unicode 逐字输入（兼容性更广）"
                onchange={(v) => (draft!.preferClipboardPaste = v)}
              />
            </div>

            <div class="mt-4">
              <div class="flex items-center justify-between gap-2">
                <p class="text-[10px] font-semibold tracking-wide text-white/40">
                  热词（{draft.hotwords.length} 个，下次识别生效）
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
              {#if draft.hotwords.length > 0}
                <div class="mt-2 flex flex-wrap gap-1.5">
                  {#each draft.hotwords as w}
                    <span
                      class="group inline-flex items-center gap-1 rounded-full bg-kotone-violet/15 py-0.5 pr-1 pl-2.5 text-[11px] text-kotone-violet ring-1 ring-kotone-violet/30"
                    >
                      {w}
                      <button
                        class="flex h-3.5 w-3.5 items-center justify-center rounded-full text-kotone-violet/60 transition hover:bg-kotone-pink/30 hover:text-kotone-pink"
                        title="删除该热词"
                        aria-label="删除热词 {w}"
                        onclick={() => removeHotword(w)}
                      >
                        ×
                      </button>
                    </span>
                  {/each}
                </div>
              {:else}
                <p class="mt-2 text-[11px] text-white/35">还没有热词——添加游戏术语可显著提升识别准确率</p>
              {/if}
              <div class="mt-2 flex items-center gap-2">
                <input
                  bind:value={newHotword}
                  placeholder="输入热词，回车添加"
                  spellcheck="false"
                  class="min-w-0 flex-1 rounded-lg bg-white/8 px-2.5 py-1.5 text-xs ring-1 ring-white/15 outline-none placeholder:text-white/30 focus:ring-kotone-cyan/60"
                  onkeydown={(e) => {
                    if (e.key === "Enter") addHotword();
                  }}
                />
                <button
                  class="shrink-0 rounded-lg bg-kotone-cyan/20 px-3 py-1.5 text-xs font-semibold text-kotone-cyan transition hover:bg-kotone-cyan/30 active:scale-95"
                  onclick={addHotword}
                >
                  添加
                </button>
              </div>
              <p class="mt-1.5 text-[10px] text-white/35">
                导入为合并模式：UTF-8 文本每行一词条，自动跳过空行与重复；导入立即保存。
              </p>
            </div>

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
</div>
