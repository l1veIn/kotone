<script lang="ts">
  // 新建 / 编辑一个已保存的自动化。程序在后端检查语法，参数声明在后端校验；这里只做基本输入。
  import { untrack } from "svelte";
  import Select from "../../../../lib/components/Select.svelte";
  import type { DeliveryKind } from "../../../../lib/assistant";
  import {
    PARAM_TYPES, saveAutomation, type Automation, type AutomationBody, type ParamSpec, type ParamType,
  } from "../../../../lib/automation";
  import { errText, toast } from "../../../../lib/stores/ui";
  import ParamInput from "./ParamInput.svelte";

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
        params: kind === "program" ? params.map((p) => ({ ...p, default: p.default ?? undefined })) : [],
        action,
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

<form class="panel" onsubmit={save} data-testid="automation-editor" aria-label={initial ? "编辑自动化" : "新建自动化"}>
  <h2>{initial ? "编辑自动化" : "新建自动化"}</h2>
  <div class="mt-3 flex flex-col gap-3">
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
        <label class="mb-1 block text-xs text-white/65" for="automation-source">程序（Luau）</label>
        <textarea id="automation-source" bind:value={source} rows="8" maxlength="8000" spellcheck="false" data-testid="automation-source"
          placeholder={'every(args.interval, function() key(args.key) end, { times = args.times })'}
          class="w-full resize-y rounded-xl border border-white/12 bg-black/25 px-3 py-2.5 font-mono text-xs text-white/90 outline-none focus:border-kotone-cyan/60"></textarea>
        <p class="mt-1 text-[11px] leading-relaxed text-white/40">
          可用：key / key_down / key_up / type_text、click / mouse_move / mouse_down / mouse_up / scroll、wait / every / wait_until、
          brightness / color_at / find_color / find_image / find_text / ocr、say / toast / log、stop。参数在程序里是 <code>args.名字</code>。
          坐标一律是窗口比例 0–1。完整说明见文档。
        </p>
      </div>

      <div>
        <div class="flex items-center justify-between">
          <h3>参数</h3>
          <button type="button" class="secondary" disabled={params.length >= 16} onclick={addParam}>添加参数</button>
        </div>
        <div class="mt-2 flex flex-col gap-2">
          {#each params as param, index (index)}
            <div class="rounded-lg bg-white/4 p-3 ring-1 ring-white/10" data-testid="automation-param">
              <div class="flex flex-wrap items-center gap-2">
                <input class="field w-28" value={param.name} aria-label={`参数 ${index + 1} 名`} placeholder="名（英文小写）"
                  onchange={(event) => patchParam(index, { name: event.currentTarget.value.trim() })} />
                <input class="field w-32" value={param.label} aria-label={`参数 ${index + 1} 显示名`} placeholder="显示名（可选）"
                  onchange={(event) => patchParam(index, { label: event.currentTarget.value })} />
                <div class="w-28"><Select ariaLabel={`参数 ${index + 1} 类型`} value={param.type}
                  options={PARAM_TYPES.map((t) => ({ value: t.id, label: t.label }))}
                  onchange={(type) => changeType(index, type as ParamType)} /></div>
                <button type="button" class="secondary ml-auto" onclick={() => { params = params.filter((_, i) => i !== index); }}>删除</button>
              </div>
              <div class="mt-2 flex flex-wrap items-center gap-3 text-[11px] text-white/55">
                {#if param.type === "number"}
                  <label class="flex items-center gap-1">最小 <input class="field w-20" type="number" value={param.min ?? ""} aria-label={`参数 ${index + 1} 最小值`}
                    onchange={(event) => patchParam(index, { min: event.currentTarget.value === "" ? null : Number(event.currentTarget.value) })} /></label>
                  <label class="flex items-center gap-1">最大 <input class="field w-20" type="number" value={param.max ?? ""} aria-label={`参数 ${index + 1} 最大值`}
                    onchange={(event) => patchParam(index, { max: event.currentTarget.value === "" ? null : Number(event.currentTarget.value) })} /></label>
                  <label class="flex items-center gap-1"><input type="checkbox" class="accent-cyan-300" checked={param.integer ?? false}
                    onchange={(event) => patchParam(index, { integer: event.currentTarget.checked })} />只能是整数</label>
                {:else if param.type === "text"}
                  <label class="flex items-center gap-1">最多 <input class="field w-20" type="number" min="1" max="200" value={param.maxLength ?? 200} aria-label={`参数 ${index + 1} 最多字数`}
                    onchange={(event) => patchParam(index, { maxLength: Number(event.currentTarget.value) })} /> 字</label>
                {:else if param.type === "choice"}
                  <label class="flex items-center gap-1">选项 <input class="field w-60" value={(param.options ?? []).join("，")} aria-label={`参数 ${index + 1} 选项`} placeholder="用逗号分隔"
                    onchange={(event) => patchParam(index, { options: event.currentTarget.value.split(/[,，]/).map((o) => o.trim()).filter(Boolean) })} /></label>
                {/if}
              </div>
              <div class="mt-2 flex flex-wrap items-center gap-2 text-[11px] text-white/55">
                <span>缺省值</span>
                <ParamInput spec={param} value={param.default ?? undefined} label={`参数 ${index + 1} 缺省值`}
                  onchange={(value) => patchParam(index, { default: value })} />
                {#if param.default === undefined || param.default === null}
                  <span class="text-white/35">没有缺省值 = 运行时必须填</span>
                {:else if param.type !== "image"}
                  <button type="button" class="text-kotone-cyan/80 hover:underline" onclick={() => patchParam(index, { default: undefined })}>清除</button>
                {/if}
              </div>
            </div>
          {/each}
        </div>
      </div>
    {:else}
      <label class="row"><span>播报内容</span>
        <input class="field min-w-0 flex-1" bind:value={text} maxlength="300" aria-label="播报内容" placeholder="如 该喝水了" /></label>
      <div class="row"><span>方式</span>
        <div class="flex gap-4 text-xs text-white/80">
          {#each [{ id: "voice", label: "语音播报" }, { id: "danmaku", label: "弹幕" }] as item}
            <label class="flex items-center gap-2"><input type="checkbox" class="accent-cyan-300"
              checked={delivery.includes(item.id as DeliveryKind)} onchange={(event) => toggleDelivery(item.id as DeliveryKind, event.currentTarget.checked)} />{item.label}</label>
          {/each}
        </div>
      </div>
    {/if}
  </div>
  {#if error}<p class="notice text-kotone-pink" role="alert">{error}</p>{/if}
  <div class="mt-4 flex justify-end gap-2">
    <button type="button" class="secondary" onclick={onCancel}>取消</button>
    <button type="submit" class="primary" disabled={saving || !name.trim()}>{saving ? "保存中…" : "保存"}</button>
  </div>
</form>

<style>
  .row { display: flex; align-items: center; gap: .75rem; }
  .row > span { width: 4.5rem; flex-shrink: 0; font-size: .75rem; color: #ffffffa6; }
  .field { border-radius: .5rem; background: #ffffff14; padding: .35rem .6rem; font-size: .75rem; outline: none; box-shadow: inset 0 0 0 1px #ffffff26; }
  .field:focus { box-shadow: inset 0 0 0 1px #00e5ff99; }
  .chip { border-radius: .5rem; padding: .3rem .65rem; font-size: .72rem; background: #ffffff0d; color: #ffffffa6; box-shadow: inset 0 0 0 1px #ffffff1a; }
  .chip[aria-pressed="true"] { background: #00e5ff26; color: #00e5ff; box-shadow: inset 0 0 0 1px #00e5ff66; }
</style>
