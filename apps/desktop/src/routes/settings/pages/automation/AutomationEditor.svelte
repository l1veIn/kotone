<script lang="ts">
  // 新建 / 编辑一个已保存的自动化。程序在后端检查语法，参数声明在后端校验；这里只做基本输入。
  import { untrack } from "svelte";
  import SelectField from "$lib/components/settings/SelectField.svelte";
  import { Button } from "$lib/components/ui/button";
  import type { DeliveryKind } from "../../../../lib/assistant";
  import {
    PARAM_TYPES, saveAutomation, type Automation, type AutomationBody, type ParamSpec, type ParamType,
  } from "../../../../lib/automation";
  import { errText, toast } from "../../../../lib/stores/ui";
  import ParamInput from "./ParamInput.svelte";
  import ArgsForm from "./ArgsForm.svelte";
  import { cronToPreset, presetToCron, type SchedulePreset } from "../../../../lib/automation";

  let { automation = null, onSaved, onCancel }: {
    automation?: Automation | null; onSaved: (saved: Automation) => void; onCancel: () => void;
  } = $props();

  // 编辑的是一份副本；保存成功才交回。
  const initial = untrack(() => automation);
  let name = $state(initial?.name ?? "");
  let description = $state(initial?.description ?? "");
  let kind = $state<AutomationBody["kind"]>(initial?.action.kind ?? "program");
  let source = $state(initial?.action.kind === "program" ? initial.action.source ?? "" : "");
  let text = $state(initial?.action.kind === "speak" ? initial.action.text : "");
  let delivery = $state<DeliveryKind[]>(initial?.action.kind === "speak" ? [...initial.action.delivery] : ["voice"]);
  // 传进来的定义可能是上层 $state 的代理，structuredClone 会抛错：用 $state.snapshot 取普通对象。
  let params = $state<ParamSpec[]>($state.snapshot(initial?.params ?? []) as ParamSpec[]);
  let saving = $state(false);
  let error = $state("");
  // 定时（类似 crontab）：常用写法用选择，其余写 cron
  const initialPreset = cronToPreset(initial?.schedule?.cron ?? "0 8 * * *");
  let scheduled = $state(initial?.schedule?.enabled ?? false);
  let presetKind = $state<SchedulePreset["kind"]>(initialPreset.kind);
  let presetTime = $state("time" in initialPreset ? initialPreset.time : "08:00");
  let presetEvery = $state("every" in initialPreset ? initialPreset.every : 10);
  let customCron = $state(initialPreset.kind === "cron" ? initialPreset.cron : (initial?.schedule?.cron ?? "0 8 * * *"));
  let scheduleArgs = $state<Record<string, unknown>>($state.snapshot(initial?.schedule?.args ?? {}) as Record<string, unknown>);
  const cron = $derived(presetToCron(
    presetKind === "daily" || presetKind === "weekdays" ? { kind: presetKind, time: presetTime }
      : presetKind === "minutes" ? { kind: "minutes", every: Number(presetEvery) }
      : presetKind === "hourly" ? { kind: "hourly" } : { kind: "cron", cron: customCron }));

  function addParam() {
    if (params.length >= 16) return;
    let n = params.length + 1;
    while (params.some((p) => p.name === `p${n}`)) n++;
    params = [...params, { name: `p${n}`, label: "", description: "", type: "number", default: 1 }];
  }

  function patchParam(index: number, patch: Partial<ParamSpec>) {
    params = params.map((p, i) => (i === index ? { ...p, ...patch } : p));
  }

  function changeType(index: number, type: ParamType) {
    // 换类型后旧的缺省值与类型专属字段不再适用
    const base = params[index];
    params = params.map((p, i) => (i === index
      ? { name: base.name, label: base.label, description: base.description, type, ...(type === "choice" ? { options: ["是", "否"] } : {}) }
      : p));
  }

  function toggleDelivery(item: DeliveryKind, checked: boolean) {
    const next = checked ? [...new Set([...delivery, item])] : delivery.filter((d) => d !== item);
    if (next.length) delivery = next;
  }

  async function save(event: SubmitEvent) {
    event.preventDefault();
    if (saving) return;
    saving = true;
    error = "";
    try {
      const action: AutomationBody = kind === "program" ? { kind, source } : { kind, text, delivery };
      const saved = await saveAutomation({
        id: initial?.id ?? null, name, description,
        params: params.map((p) => ({ ...p, default: p.default ?? undefined })),
        action,
        // 关掉定时时保留写好的时间（暂停），下次打开还在
        schedule: scheduled || initial?.schedule ? { cron, enabled: scheduled, args: $state.snapshot(scheduleArgs) as Record<string, unknown> } : null,
      });
      toast(true, initial ? "自动化已保存" : "已新建自动化");
      onSaved(saved);
    } catch (err) {
      error = errText(err);
    } finally {
      saving = false;
    }
  }
</script>

{#snippet paramsEditor()}
      <div>
        <div class="flex items-center justify-between">
          <h3 class="m-0 text-[13px] font-medium">参数</h3>
          <Button variant="outline" size="sm" disabled={params.length >= 16} onclick={addParam}>添加参数</Button>
        </div>
        <div class="mt-2 flex flex-col gap-2">
          {#each params as param, index (index)}
            <div class="rounded-xl border border-hairline bg-muted/40 p-3" data-testid="automation-param">
              <div class="flex flex-wrap items-center gap-2">
                <input class="field w-28" value={param.name} aria-label={`参数 ${index + 1} 名`} placeholder="名（英文小写）"
                  onchange={(event) => patchParam(index, { name: event.currentTarget.value.trim() })} />
                <input class="field w-32" value={param.label} aria-label={`参数 ${index + 1} 显示名`} placeholder="显示名（可选）"
                  onchange={(event) => patchParam(index, { label: event.currentTarget.value })} />
                <div class="w-28"><SelectField ariaLabel={`参数 ${index + 1} 类型`} value={param.type}
                  options={PARAM_TYPES.map((t) => ({ value: t.id, label: t.label }))}
                  onchange={(type) => changeType(index, type as ParamType)} /></div>
                <Button variant="ghost" size="sm" class="ml-auto text-muted-foreground hover:text-destructive" onclick={() => { params = params.filter((_, i) => i !== index); }}>删除</Button>
              </div>
              <div class="mt-2 flex flex-wrap items-center gap-3 text-xs text-muted-foreground">
                {#if param.type === "number"}
                  <label class="flex items-center gap-1">最小 <input class="field w-20" type="number" value={param.min ?? ""} aria-label={`参数 ${index + 1} 最小值`}
                    onchange={(event) => patchParam(index, { min: event.currentTarget.value === "" ? null : Number(event.currentTarget.value) })} /></label>
                  <label class="flex items-center gap-1">最大 <input class="field w-20" type="number" value={param.max ?? ""} aria-label={`参数 ${index + 1} 最大值`}
                    onchange={(event) => patchParam(index, { max: event.currentTarget.value === "" ? null : Number(event.currentTarget.value) })} /></label>
                  <label class="flex items-center gap-1.5"><input type="checkbox" class="size-4 accent-[var(--color-primary)]" checked={param.integer ?? false}
                    onchange={(event) => patchParam(index, { integer: event.currentTarget.checked })} />只能是整数</label>
                {:else if param.type === "text"}
                  <label class="flex items-center gap-1">最多 <input class="field w-20" type="number" min="1" max="200" value={param.maxLength ?? 200} aria-label={`参数 ${index + 1} 最多字数`}
                    onchange={(event) => patchParam(index, { maxLength: Number(event.currentTarget.value) })} /> 字</label>
                {:else if param.type === "choice"}
                  <label class="flex items-center gap-1">选项 <input class="field w-60" value={(param.options ?? []).join("，")} aria-label={`参数 ${index + 1} 选项`} placeholder="用逗号分隔"
                    onchange={(event) => patchParam(index, { options: event.currentTarget.value.split(/[,，]/).map((o) => o.trim()).filter(Boolean) })} /></label>
                {/if}
              </div>
              <div class="mt-2 flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
                <span>缺省值</span>
                <ParamInput spec={param} value={param.default ?? undefined} label={`参数 ${index + 1} 缺省值`}
                  onchange={(value) => patchParam(index, { default: value })} />
                {#if param.default === undefined || param.default === null}
                  <span class="text-muted-foreground/80">没有缺省值 = 运行时必须填</span>
                {:else if param.type !== "image"}
                  <button type="button" class="text-primary hover:underline" onclick={() => patchParam(index, { default: undefined })}>清除</button>
                {/if}
              </div>
            </div>
          {/each}
        </div>
      </div>
{/snippet}

<form class="rounded-2xl border border-hairline bg-card/70 p-5 text-foreground" onsubmit={save} data-testid="automation-editor" aria-label={initial ? "编辑自动化" : "新建自动化"}>
  <h2 class="m-0 text-[15px] font-semibold">{initial ? "编辑自动化" : "新建自动化"}</h2>
  <div class="mt-4 flex flex-col gap-3.5">
    <label class="row"><span>名字</span>
      <input class="field w-64 max-w-full" bind:value={name} maxlength="40" aria-label="自动化名字" placeholder="如 每秒按 Q" /></label>
    <label class="row"><span>说明</span>
      <input class="field min-w-0 flex-1" bind:value={description} maxlength="200" aria-label="自动化说明" placeholder="可选" /></label>
    <div class="row"><span>做什么</span>
      <div class="flex gap-1.5" role="group" aria-label="自动化类型">
        {#each [{ id: "program", label: "运行一段程序" }, { id: "speak", label: "播报一句话" }] as option}
          <button type="button" class="chip" aria-pressed={kind === option.id} onclick={() => { kind = option.id as AutomationBody["kind"]; }}>{option.label}</button>
        {/each}
      </div>
    </div>

    {#if kind === "program"}
      <div>
        <label class="mb-1.5 block text-xs font-medium text-muted-foreground" for="automation-source">程序（Luau）</label>
        <textarea id="automation-source" bind:value={source} rows="8" maxlength="8000" spellcheck="false" data-testid="automation-source"
          placeholder={'every(args.interval, function() key(args.key) end, { times = args.times })'}
          class="w-full resize-y rounded-xl border border-input bg-background/60 px-3 py-2.5 font-mono text-xs leading-5 text-foreground outline-none transition-colors focus:border-ring focus:ring-3 focus:ring-ring/30"></textarea>
        <p class="mt-1.5 mb-0 text-[11px] leading-relaxed text-muted-foreground">
          可用：key / key_down / key_up / type_text、click / mouse_move / mouse_down / mouse_up / scroll、wait / every / wait_until、
          brightness / color_at / find_color / find_image / find_text / ocr、say / toast / log、stop。参数在程序里是 <code>args.名字</code>。
          坐标一律是窗口比例 0–1。完整说明见文档。
        </p>
      </div>

      {@render paramsEditor()}
    {:else}
      <label class="row"><span>播报内容</span>
        <input class="field min-w-0 flex-1" bind:value={text} maxlength="300" aria-label="播报内容" placeholder={"如 该喝水了，或 该{text}了"} /></label>
      <p class="m-0 -mt-1.5 pl-[5.25rem] text-[11px] text-muted-foreground">内容里写 {"{参数名}"} 会换成运行时填的值；加一个数字参数 delay_seconds 就是“几秒后再说”。</p>
      <div class="row"><span>方式</span>
        <div class="flex gap-4 text-[13px] text-foreground">
          {#each [{ id: "voice", label: "语音播报" }, { id: "danmaku", label: "弹幕" }] as item}
            <label class="flex items-center gap-2"><input type="checkbox" class="size-4 accent-[var(--color-primary)]"
              checked={delivery.includes(item.id as DeliveryKind)} onchange={(event) => toggleDelivery(item.id as DeliveryKind, event.currentTarget.checked)} />{item.label}</label>
          {/each}
        </div>
      </div>
      {@render paramsEditor()}
    {/if}

    <div class="rounded-xl border border-hairline bg-muted/30 p-3" data-testid="automation-schedule">
      <label class="flex items-center gap-2 text-[13px] font-medium">
        <input type="checkbox" class="size-4 accent-[var(--color-primary)]" bind:checked={scheduled} data-testid="automation-schedule-enabled" />定时运行
        <span class="font-normal text-xs text-muted-foreground">（像 crontab：到点自己运行一次；Kotone 没开时错过的不补）</span>
      </label>
      {#if scheduled}
        <div class="mt-2.5 flex flex-wrap items-center gap-2 text-[13px]">
          <div class="w-40"><SelectField ariaLabel="定时方式" value={presetKind}
            options={[{ value: "daily", label: "每天" }, { value: "weekdays", label: "工作日（周一到周五）" }, { value: "hourly", label: "每小时整点" }, { value: "minutes", label: "每隔几分钟" }, { value: "cron", label: "自定义（cron）" }]}
            onchange={(value) => { presetKind = value as SchedulePreset["kind"]; }} /></div>
          {#if presetKind === "daily" || presetKind === "weekdays"}
            <input class="field w-28" type="time" bind:value={presetTime} aria-label="几点" data-testid="automation-schedule-time" />
          {:else if presetKind === "minutes"}
            每 <input class="field w-16" type="number" min="1" max="59" bind:value={presetEvery} aria-label="每隔几分钟" /> 分钟
          {:else if presetKind === "cron"}
            <input class="field w-44 font-mono" bind:value={customCron} aria-label="cron" placeholder="分 时 日 月 周，如 30 21 * * 5" data-testid="automation-schedule-cron" />
          {/if}
          <span class="text-xs text-muted-foreground">= <code>{cron}</code></span>
        </div>
        {#if params.length}
          <div class="mt-3">
            <p class="m-0 mb-1.5 text-xs text-muted-foreground">到点运行时用的参数</p>
            <ArgsForm params={params} values={scheduleArgs} prefix="定时 " onchange={(values) => { scheduleArgs = values; }} />
          </div>
        {/if}
      {/if}
    </div>
  </div>
  {#if error}<p class="mt-3 mb-0 text-xs text-destructive" role="alert">{error}</p>{/if}
  <div class="mt-5 flex justify-end gap-2">
    <Button variant="outline" size="sm" onclick={onCancel}>取消</Button>
    <Button type="submit" size="sm" disabled={saving || !name.trim()}>{saving ? "保存中…" : "保存"}</Button>
  </div>
</form>

<style>
  .row { display: flex; align-items: center; gap: .75rem; }
  .row > span { width: 4.5rem; flex-shrink: 0; font-size: 13px; color: var(--color-muted-foreground); }
  .field { height: 2rem; border-radius: .5rem; border: 1px solid var(--color-input); background: color-mix(in oklab, var(--color-background) 60%, transparent); color: var(--color-foreground); padding: 0 .6rem; font-size: 13px; outline: none; transition: border-color 120ms, box-shadow 120ms; }
  .field::placeholder { color: var(--color-muted-foreground); }
  .field:focus { border-color: var(--color-ring); box-shadow: 0 0 0 3px color-mix(in oklab, var(--color-ring) 30%, transparent); }
  .chip { height: 1.75rem; border-radius: .5rem; padding: 0 .7rem; font-size: 12px; color: var(--color-muted-foreground); border: 1px solid var(--color-hairline); transition: background 120ms, color 120ms; }
  .chip:hover { color: var(--color-foreground); background: color-mix(in oklab, var(--color-muted) 60%, transparent); }
  .chip[aria-pressed="true"] { background: color-mix(in oklab, var(--color-primary) 12%, transparent); color: var(--color-primary); border-color: color-mix(in oklab, var(--color-primary) 50%, transparent); }
</style>
