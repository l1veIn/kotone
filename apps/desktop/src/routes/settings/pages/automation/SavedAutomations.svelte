<script lang="ts">
  // 已保存的自动化：列表、运行（填参数 + 倒数切到游戏）、编辑、删除、新建。
  import {
    deleteAutomation, getAutomation, runAutomation, type Automation,
  } from "../../../../lib/automation";
  import { agentSetAutomationHidden } from "../../../../lib/agent";
  import { errText, settingsStore, toast } from "../../../../lib/stores/ui";
  import { Switch } from "$lib/components/ui/switch";
  import { Button } from "$lib/components/ui/button";
  import { Play, Plus } from "@lucide/svelte";
  import SettingsSection from "$lib/components/settings/SettingsSection.svelte";
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
  /** 不交给琴音（内置 Agent）的自动化；默认全部交给，最多 30 个（ADR 014）。 */
  const hiddenFromAgent = $derived(new Set($settingsStore?.agent?.hiddenAutomations ?? []));
  let offering = $state("");

  async function setOffered(item: Automation, offered: boolean) {
    if (offering) return;
    offering = item.id;
    try {
      const hiddenAutomations = await agentSetAutomationHidden(item.id, !offered);
      settingsStore.update((s) => s && { ...s, agent: { permissions: { mode: "default", rules: {} }, inGame: true, ...s.agent, hiddenAutomations } });
      toast(true, offered ? `「${item.name}」交给琴音使用` : `「${item.name}」不再交给琴音`);
    } catch (error) { toast(false, `保存失败：${errText(error)}`); }
    finally { offering = ""; }
  }

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
  <SettingsSection
    title={`已保存的自动化 · ${automations.length}`}
    id="saved-automations-title"
    description="保存下来的自动化直接运行，不再调用 AI。可以在「琴音陪玩 → 设置」的快捷指令里绑到热键上：在游戏里按下就运行。打开「交给琴音」的（最多 30 个）琴音也能帮你运行。"
    flush
  >
    {#snippet actions()}
      <Button size="sm" onclick={() => { editing = "new"; }}><Plus class="size-3.5" />新建</Button>
    {/snippet}
    {#if !automations.length}
      <p class="m-0 px-4 py-8 text-center text-[13px] text-muted-foreground">还没有保存的自动化</p>
    {:else}
      <ul class="m-0 list-none divide-y divide-hairline p-0">
        {#each automations as item (item.id)}
          <li class="px-4 py-3.5" data-testid="saved-automation">
            <div class="flex flex-wrap items-start justify-between gap-3">
              <div class="min-w-0">
                <p class="m-0 text-[13px] font-medium text-foreground">{item.name}
                  <span class="ml-1.5 text-[11px] font-normal text-muted-foreground">{item.action.kind === "speak" ? "播报" : "程序"} · {by[item.createdBy]}创建</span></p>
                {#if item.description}<p class="mt-1 mb-0 text-xs text-muted-foreground">{item.description}</p>{/if}
                {#if item.params.length}
                  <p class="mt-1 mb-0 text-[11px] text-muted-foreground">参数：{item.params.map((p) => p.label || p.name).join("、")}</p>
                {/if}
                <label class="mt-2 flex w-fit cursor-pointer items-center gap-1.5 text-xs text-muted-foreground">
                  <Switch size="sm" data-testid="saved-automation-agent" checked={!hiddenFromAgent.has(item.id)} disabled={offering === item.id}
                    aria-label={`把「${item.name}」交给琴音使用`} onCheckedChange={(checked) => void setOffered(item, checked)} />
                  交给琴音
                </label>
              </div>
              <div class="flex shrink-0 gap-1.5">
                <Button variant="outline" size="sm" onclick={() => openRun(item)}><Play class="size-3 fill-current" />运行</Button>
                <Button variant="ghost" size="sm" onclick={() => edit(item)}>编辑</Button>
                <Button variant={confirmDelete === item.id ? "destructive" : "ghost"} size="sm"
                  class={confirmDelete === item.id ? "" : "text-muted-foreground hover:text-destructive"}
                  onclick={() => remove(item)}>{confirmDelete === item.id ? "确认删除" : "删除"}</Button>
              </div>
            </div>
            {#if runningId === item.id}
              <div class="mt-3 rounded-xl border border-hairline bg-muted/40 p-3" data-testid="automation-run-panel">
                <ArgsForm params={item.params} values={runArgs} prefix={`${item.name} `} onchange={(values) => { runArgs = values; }} />
                <div class="mt-3 flex flex-wrap items-center justify-end gap-3">
                  {#if item.action.kind === "program"}
                    <label class="flex items-center gap-1.5 text-xs text-muted-foreground">
                      点击后
                      <input class="h-7 w-14 rounded-lg border border-input bg-background/60 px-2 text-xs text-foreground outline-none focus:border-ring" type="number" min="1" max="30" bind:value={switchSeconds} aria-label="倒数秒数" />
                      秒在当时的前台窗口（切到游戏）开始
                    </label>
                  {/if}
                  <Button size="sm" disabled={busy} onclick={() => run(item)}>{busy ? "启动中…" : "开始"}</Button>
                </div>
              </div>
            {/if}
          </li>
        {/each}
      </ul>
    {/if}
  </SettingsSection>
{/if}
