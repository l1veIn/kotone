<script lang="ts">
  // 已保存的自动化：列表、运行（填参数 + 倒数切到游戏）、编辑、删除、新建。
  import {
    deleteAutomation, getAutomation, runAutomation, type Automation,
  } from "../../../../lib/automation";
  import { errText, toast } from "../../../../lib/stores/ui";
  import AutomationEditor from "./AutomationEditor.svelte";
  import ArgsForm from "./ArgsForm.svelte";

  let { automations, onChanged }: { automations: Automation[]; onChanged: () => void } = $props();

  let editing = $state<Automation | null | "new">(null);
  let runningId = $state("");
  let runArgs = $state<Record<string, unknown>>({});
  let switchSeconds = $state(3);
  let busy = $state(false);
  let confirmDelete = $state("");

  const by: Record<Automation["createdBy"], string> = { user: "你", companion: "陪玩", agent: "Agent" };

  async function edit(item: Automation) {
    // 列表不含源码，编辑前取完整定义
    try { editing = await getAutomation(item.id); }
    catch (error) { toast(false, `读取失败：${errText(error)}`); }
  }

  function openRun(item: Automation) {
    runningId = runningId === item.id ? "" : item.id;
    runArgs = {};
  }

  async function run(item: Automation) {
    if (busy) return;
    busy = true;
    try {
      const result = await runAutomation(item.id, runArgs, switchSeconds);
      toast(true, result.status === "countdown"
        ? `${result.seconds} 秒后开始，请切到游戏窗口`
        : "已开始");
      runningId = "";
    } catch (error) {
      toast(false, `没能运行：${errText(error)}`);
    } finally {
      busy = false;
    }
  }

  async function remove(item: Automation) {
    if (confirmDelete !== item.id) { confirmDelete = item.id; return; }
    confirmDelete = "";
    try {
      await deleteAutomation(item.id);
      toast(true, "已删除");
      onChanged();
    } catch (error) { toast(false, `删除失败：${errText(error)}`); }
  }
</script>

{#if editing}
  <AutomationEditor automation={editing === "new" ? null : editing}
    onSaved={() => { editing = null; onChanged(); }} onCancel={() => { editing = null; }} />
{:else}
  <section class="panel" aria-labelledby="saved-automations-title">
    <div class="flex items-center justify-between gap-3">
      <h2 id="saved-automations-title">已保存的自动化 <span class="ml-1 text-kotone-cyan">{automations.length}</span></h2>
      <button type="button" class="primary" onclick={() => { editing = "new"; }}>新建</button>
    </div>
    <p class="mt-2 text-xs leading-relaxed text-white/45">
      保存下来的自动化直接运行，不再调用 AI。可以在「陪玩」的快捷指令里绑到热键上：在游戏里按下就运行。
    </p>
    {#if !automations.length}
      <p class="empty">还没有保存的自动化</p>
    {:else}
      <ul class="mt-4 space-y-2">
        {#each automations as item (item.id)}
          <li class="rounded-xl bg-black/12 p-3" data-testid="saved-automation">
            <div class="flex flex-wrap items-start justify-between gap-3">
              <div class="min-w-0">
                <p class="text-sm text-white/90">{item.name}
                  <span class="ml-1 text-[11px] text-white/40">{item.action.kind === "speak" ? "播报" : "程序"} · {by[item.createdBy]}创建</span></p>
                {#if item.description}<p class="mt-1 text-xs text-white/50">{item.description}</p>{/if}
                {#if item.params.length}
                  <p class="mt-1 text-[11px] text-white/40">参数：{item.params.map((p) => p.label || p.name).join("、")}</p>
                {/if}
              </div>
              <div class="flex shrink-0 gap-2">
                <button type="button" class="secondary" onclick={() => openRun(item)}>运行</button>
                <button type="button" class="secondary" onclick={() => edit(item)}>编辑</button>
                <button type="button" class="secondary" onclick={() => remove(item)}>{confirmDelete === item.id ? "确认删除" : "删除"}</button>
              </div>
            </div>
            {#if runningId === item.id}
              <div class="mt-3 border-t border-white/8 pt-3" data-testid="automation-run-panel">
                <ArgsForm params={item.params} values={runArgs} prefix={`${item.name} `} onchange={(values) => { runArgs = values; }} />
                <div class="mt-3 flex flex-wrap items-center justify-end gap-3">
                  {#if item.action.kind === "program"}
                    <label class="flex items-center gap-1.5 text-[11px] text-white/55">
                      点击后
                      <input class="w-14 rounded bg-white/8 px-2 py-1 text-xs" type="number" min="1" max="30" bind:value={switchSeconds} aria-label="倒数秒数" />
                      秒在当时的前台窗口（切到游戏）开始
                    </label>
                  {/if}
                  <button type="button" class="primary" disabled={busy} onclick={() => run(item)}>{busy ? "启动中…" : "开始"}</button>
                </div>
              </div>
            {/if}
          </li>
        {/each}
      </ul>
    {/if}
  </section>
{/if}
