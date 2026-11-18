<script lang="ts">
  // 「琴音 → 角色」：可切换的角色（琴音是默认的）。每个角色各一份名字、头像、性格（SOUL）、声音、唤醒词；
  // 关于你的记忆、游戏记忆、技能和设置所有角色共用。换角色会开一段新对话（系统提示在一段对话里不变）。
  // 角色卡格式照 Character Card V2（导出 / 导入 .json）；唤醒词照 sherpa-onnx text2token 转成拼音并检查。
  import { onMount } from "svelte";
  import { Check, Download, Plus, Upload } from "@lucide/svelte";
  import { Button } from "$lib/components/ui/button";
  import { Input } from "$lib/components/ui/input";
  import SettingsSection from "$lib/components/settings/SettingsSection.svelte";
  import SettingsRow from "$lib/components/settings/SettingsRow.svelte";
  import SelectField from "$lib/components/settings/SelectField.svelte";
  import KotoneAvatar from "../../../../lib/components/KotoneAvatar.svelte";
  import { listSpeechVoices, type SpeechVoice } from "../../../../lib/assistant";
  import {
    DEFAULT_ROLE, FACE_CREDITS, bundledFaces, placeholderLetter, rolesCheckWake, rolesCreate, rolesDelete, rolesExport, rolesGet, rolesImport, rolesList,
    rolesSwitch, rolesUpdate, type RoleInfo, type WakeWordCheck,
  } from "../../../../lib/roles";
  import { workspaceAvatar, workspaceSetAvatar } from "../../../../lib/workspace";
  import { errText, settingsStore, toast } from "../../../../lib/stores/ui";
  import { getSettings } from "../../../../lib/ipc";

  let { onOpenTalk }: { onOpenTalk: () => void } = $props();

  const NAME_LIMIT = 20;
  const SOUL_LIMIT = 4000;
  let roles = $state<RoleInfo[]>([]);
  let selectedId = $state<string>(DEFAULT_ROLE);
  let role = $state<RoleInfo | null>(null);
  let avatars = $state<Record<string, string | null>>({});
  let voices = $state<SpeechVoice[]>([]);
  let busy = $state(false);
  let error = $state("");
  // 编辑中的值
  let name = $state("");
  let summary = $state("");
  let soul = $state("");
  let wakeText = $state("");
  let readings = $state<string[]>([]);
  let wakeCheck = $state<WakeWordCheck | null>(null);
  let avatarError = $state("");
  let fileInput = $state<HTMLInputElement>();
  let cardInput = $state<HTMLInputElement>();
  let newName = $state("");

  const soulChars = $derived([...soul].length);
  /** 模板里用（只在 {#if role} 里面用，那里 role 一定有值）。 */
  const cur = $derived(role as RoleInfo);
  /** 空着时用的唤醒词（和后端 wake_word::default_for_name 一样：两个字的名字重复两遍）。 */
  const defaultWake = $derived.by(() => {
    if (!role) return "";
    if (!role.wake.text) return role.wakeCheck.text;
    if (role.id === DEFAULT_ROLE && role.name === "琴音") return "琴音琴音";
    const chars = [...role.name.trim()];
    return chars.length === 1 ? `小${role.name}小${role.name}` : chars.length === 2 ? role.name + role.name : role.name;
  });

  async function refresh(select?: string) {
    try {
      const list = await rolesList();
      roles = list.roles;
      if (select) selectedId = select;
      else if (!roles.some((r) => r.id === selectedId)) selectedId = list.current;
      await load(selectedId);
      void loadAvatars();
      error = "";
    } catch (e) { error = errText(e); }
  }
  async function load(id: string) {
    const r = await rolesGet(id);
    role = r;
    name = r.name;
    summary = r.summary;
    soul = r.soul ?? "";
    wakeText = r.wake.text;
    readings = [...r.wake.readings];
    wakeCheck = r.wakeCheck;
  }
  async function loadAvatars() {
    const next: Record<string, string | null> = {};
    for (const r of roles) {
      const bytes = r.avatar ? await workspaceAvatar(r.avatar).catch(() => null) : null;
      next[r.id] = bytes ? URL.createObjectURL(new Blob([bytes as BlobPart])) : null;
    }
    Object.values(avatars).forEach((u) => u && URL.revokeObjectURL(u));
    avatars = next;
  }
  onMount(() => {
    void refresh();
    void listSpeechVoices().then((v) => { voices = v; }).catch(() => {});
    return () => Object.values(avatars).forEach((u) => u && URL.revokeObjectURL(u));
  });

  async function run(action: () => Promise<unknown>, message?: string) {
    if (busy) return;
    busy = true;
    try {
      await action();
      // 名字、头像、当前角色在设置里（页标题、侧栏、气泡都读它）：读一遍最新的。
      settingsStore.set(await getSettings());
      if (message) toast(true, message);
    }
    catch (e) { toast(false, errText(e)); }
    finally { busy = false; }
  }
  function select(id: string) {
    selectedId = id;
    void load(id).catch((e) => { error = errText(e); });
  }
  const switchTo = (r: RoleInfo) => run(async () => { await rolesSwitch(r.id); await refresh(r.id); }, `换成「${r.name}」了，下一句话开始是新对话`);
  const saveName = () => run(async () => {
    if (!role) return;
    const next = name.trim();
    if (!next || [...next].length > NAME_LIMIT) throw new Error(`名字不能为空、不超过 ${NAME_LIMIT} 个字`);
    await rolesUpdate(role.id, { name: next, summary: summary.trim() });
    await refresh(role.id);
  }, "已保存");
  const saveSoul = () => run(async () => {
    if (!role || soulChars > SOUL_LIMIT) return;
    await rolesUpdate(role.id, { soul });
    await refresh(role.id);
  }, "性格已保存，下一段对话生效");
  const saveVoice = (voice: string) => run(async () => {
    if (!role) return;
    await rolesUpdate(role.id, { voice });
    await refresh(role.id);
  }, "声音已保存");

  // 唤醒词：边打字边检查（拼音、读音、模型认不认）。
  let checkTimer: ReturnType<typeof setTimeout> | undefined;
  function recheck() {
    clearTimeout(checkTimer);
    checkTimer = setTimeout(() => {
      const text = wakeText.trim() || defaultWake;
      void rolesCheckWake(text, readings).then((c) => { wakeCheck = c; }).catch(() => {});
    }, 200);
  }
  function pickReading(index: number, value: string) {
    const next = [...readings];
    while (next.length <= index) next.push("");
    next[index] = value;
    readings = next;
    recheck();
  }
  const saveWake = () => run(async () => {
    if (!role) return;
    if (wakeText.trim() && !wakeCheck?.ok) throw new Error(wakeCheck?.problems.join("；") || "唤醒词不能用");
    await rolesUpdate(role.id, { wake: { text: wakeText.trim(), readings } });
    await refresh(role.id);
  }, "唤醒词已保存（开着语音唤醒时立刻换成新的）");

  async function pickAvatar(event: Event) {
    const input = event.currentTarget as HTMLInputElement;
    const file = input.files?.[0];
    input.value = "";
    if (!file || !role) return;
    avatarError = "";
    if (file.size > 2 * 1024 * 1024) { avatarError = "头像不能超过 2 MB"; return; }
    const ext = file.name.split(".").pop() ?? "";
    const id = role.id;
    await run(async () => {
      const saved = await workspaceSetAvatar(new Uint8Array(await file.arrayBuffer()), ext, id);
      await rolesUpdate(id, { avatar: saved });
      await refresh(id);
    }, "头像已更新").catch((e) => { avatarError = errText(e); });
  }
  const clearAvatar = () => run(async () => { if (role) { await rolesUpdate(role.id, { avatar: "" }); await refresh(role.id); } }, "已恢复默认形象");

  const createRole = () => run(async () => {
    const r = await rolesCreate({ name: newName.trim() });
    newName = "";
    await refresh(r.id);
  }, "角色已新建");
  const removeRole = () => run(async () => {
    if (!role) return;
    const id = role.id;
    await rolesDelete(id);
    await refresh(role.builtin ? id : undefined);
  }, role?.builtin ? "已恢复默认" : "角色已删除");
  const exportRole = () => run(async () => {
    if (!role) return;
    const card = await rolesExport(role.id);
    const url = URL.createObjectURL(new Blob([JSON.stringify(card, null, 2)], { type: "application/json" }));
    const a = document.createElement("a");
    a.href = url;
    a.download = `${role.name}.card.json`;
    a.click();
    setTimeout(() => URL.revokeObjectURL(url), 1000);
  }, "已导出角色卡");
  async function importCard(event: Event) {
    const input = event.currentTarget as HTMLInputElement;
    const file = input.files?.[0];
    input.value = "";
    if (!file) return;
    await run(async () => {
      const card = JSON.parse(await file.text());
      const r = await rolesImport(card);
      await refresh(r.id);
    }, "角色卡已导入");
  }

  const voiceOptions = $derived([{ value: "", label: "跟随「回答」里的声音" }, ...voices.map((v) => ({ value: v.id, label: v.name }))]);
</script>

<div class="space-y-6" data-testid="identity-tab">
  <SettingsSection title="角色" description="每个角色有自己的名字、头像、性格、声音和唤醒词；关于你的记忆、游戏记忆、技能和设置所有角色共用，换角色不会忘掉你。换角色会开一段新对话。">
    {#if error}<p role="alert" class="m-0 text-xs text-destructive">{error}</p>{/if}
    <div class="grid gap-2 py-3 sm:grid-cols-2 lg:grid-cols-3" data-testid="role-list">
      {#each roles as r (r.id)}
        <div role="button" tabindex="0" data-testid="role-card" data-role={r.id} aria-pressed={r.id === selectedId}
          class="flex cursor-pointer items-center gap-3 rounded-xl border p-3 text-left transition-colors {r.id === selectedId ? 'border-primary/60 bg-primary/5' : 'border-border/80 hover:bg-muted/40'}"
          onclick={() => select(r.id)} onkeydown={(e) => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); select(r.id); } }}>
          <KotoneAvatar size={40} mood="asking" custom={avatars[r.id] ?? null} faces={bundledFaces(r.id, r.avatar)} letter={placeholderLetter(r.id, r.name, r.avatar)} />
          <div class="min-w-0 flex-1">
            <div class="flex min-w-0 items-center gap-1.5 text-sm font-medium"><span class="truncate">{r.name}</span>{#if r.current}<span class="shrink-0 rounded-full bg-primary/15 px-1.5 text-[11px] text-primary" data-testid="role-current">当前</span>{/if}</div>
            <p class="m-0 truncate text-xs text-muted-foreground">{r.summary || "自己建的角色"}</p>
          </div>
          {#if !r.current}
            <Button size="sm" variant="outline" class="shrink-0" disabled={busy} aria-label={`换成${r.name}`} title={`换成${r.name}`} onclick={(e) => { e.stopPropagation(); void switchTo(r); }} data-testid="role-switch">切换</Button>
          {/if}
        </div>
      {/each}
    </div>
    <div class="flex flex-wrap items-center gap-2 pb-2">
      <Input class="h-8 w-40" bind:value={newName} maxlength={NAME_LIMIT} placeholder="新角色的名字" data-testid="role-new-name"
        onkeydown={(e) => { if (e.key === "Enter" && newName.trim()) void createRole(); }} />
      <Button size="sm" disabled={busy || !newName.trim()} onclick={() => void createRole()} data-testid="role-create"><Plus class="size-3.5" />新建角色</Button>
      <input bind:this={cardInput} type="file" accept=".json,application/json" class="hidden" onchange={(e) => void importCard(e)} data-testid="role-import-input" />
      <Button size="sm" variant="ghost" disabled={busy} onclick={() => cardInput?.click()}><Upload class="size-3.5" />导入角色卡</Button>
    </div>
  </SettingsSection>

  {#if role}
    <SettingsSection title={`编辑：${cur.name}`} description={cur.builtin ? "内置角色：改了的部分存在你的工作区，随时可以恢复默认。" : "你自己建的角色。"}>
      <SettingsRow label="名字" for="role-name" description={`最多 ${NAME_LIMIT} 个字；她也会用这个名字称呼自己`}>
        {#snippet control()}
          <div class="flex flex-wrap items-center gap-2">
            <Input id="role-name" class="h-8 w-36" bind:value={name} maxlength={NAME_LIMIT} data-testid="identity-name"
              onkeydown={(e) => { if (e.key === "Enter") void saveName(); }} />
            <Input class="h-8 w-48" bind:value={summary} maxlength={60} placeholder="一句话介绍" data-testid="role-summary" />
            <Button size="sm" disabled={busy || (name.trim() === cur.name && summary.trim() === cur.summary)} onclick={() => void saveName()} data-testid="identity-name-save">保存</Button>
          </div>
        {/snippet}
      </SettingsRow>
      <SettingsRow label="头像" description={cur.id === DEFAULT_ROLE ? "PNG、JPG、WebP 或 GIF，不超过 2 MB；不传就用琴音的立绘和表情" : "PNG、JPG、WebP 或 GIF，不超过 2 MB；不传就显示名字的第一个字"}>
        {#snippet control()}
          <div class="flex items-center gap-3">
            <div data-testid="identity-avatar" data-src={avatars[cur.id] ?? ""}><KotoneAvatar size={48} mood="asking" custom={avatars[cur.id] ?? null} faces={bundledFaces(cur.id, cur.avatar)} letter={placeholderLetter(cur.id, cur.name, cur.avatar)} /></div>
            <input bind:this={fileInput} type="file" accept=".png,.jpg,.jpeg,.webp,.gif,image/png,image/jpeg,image/webp,image/gif" class="hidden"
              onchange={(e) => void pickAvatar(e)} data-testid="identity-avatar-input" />
            <Button size="sm" variant="outline" disabled={busy} onclick={() => fileInput?.click()}>换一张</Button>
            {#if cur.avatar}<Button size="sm" variant="ghost" disabled={busy} onclick={() => void clearAvatar()}>用默认</Button>{/if}
          </div>
        {/snippet}
        {#if avatarError}<p role="alert" class="m-0 text-xs text-destructive" data-testid="identity-avatar-error">{avatarError}</p>{/if}
        {#if FACE_CREDITS[cur.id] && !cur.avatar}<p class="m-0 text-[11px] leading-relaxed text-muted-foreground" data-testid="role-face-credit">{FACE_CREDITS[cur.id]}</p>{/if}
      </SettingsRow>
      <SettingsRow label="声音" description="她说话用的声音（属于当前语音引擎的才生效，见「回答 → 语音」）">
        {#snippet control()}
          <div class="w-60 max-w-full"><SelectField ariaLabel="角色的声音" value={cur.voice} options={voiceOptions.some((o) => o.value === cur.voice) ? voiceOptions : [...voiceOptions, { value: cur.voice, label: cur.voice }]}
            disabled={busy} onchange={(v) => void saveVoice(v)} testid="role-voice" /></div>
        {/snippet}
      </SettingsRow>
      <SettingsRow label="唤醒词" for="role-wake" description={`喊这个叫醒她（开着「语音唤醒」时）。空着用默认：「${defaultWake}」。推荐 4 个字，如名字重复两遍。`}>
        {#snippet control()}
          <div class="flex items-center gap-2">
            <Input id="role-wake" class="h-8 w-40" bind:value={wakeText} maxlength={8} placeholder={defaultWake} data-testid="role-wake"
              oninput={() => { readings = []; recheck(); }} />
            <Button size="sm" disabled={busy || (wakeText.trim() === cur.wake.text && readings.join() === cur.wake.readings.join())} onclick={() => void saveWake()} data-testid="role-wake-save">保存</Button>
          </div>
        {/snippet}
        {#if wakeCheck}
          <div class="mt-1 space-y-1" data-testid="role-wake-check" data-ok={wakeCheck.ok}>
            <div class="flex flex-wrap gap-1.5">
              {#each wakeCheck.syllables as s, i (i)}
                <span class="inline-flex flex-col items-center rounded-md border px-1.5 py-0.5 text-xs {s.missing.length ? 'border-destructive/60 text-destructive' : 'border-border/80'}">
                  <span class="text-sm">{s.character}</span>
                  {#if s.choices.length > 1}
                    <select class="bg-transparent text-[11px] text-primary" aria-label={`“${s.character}”的读音`} value={s.reading}
                      onchange={(e) => pickReading(i, e.currentTarget.value)} data-testid="role-wake-reading">
                      {#each s.choices as c}<option value={c}>{c}</option>{/each}
                    </select>
                  {:else}<span class="text-[11px] text-muted-foreground">{s.reading}</span>{/if}
                </span>
              {/each}
            </div>
            {#if wakeCheck.ok}<p class="m-0 flex items-center gap-1 text-xs text-success"><Check class="size-3" />唤醒模型认得这个词{#if wakeCheck.warnings.length}<span class="text-warning">（{wakeCheck.warnings.join("；")}）</span>{/if}</p>
            {:else}<p class="m-0 text-xs text-destructive" role="alert" data-testid="role-wake-problem">{wakeCheck.problems.join("；")}</p>{/if}
            <p class="m-0 text-[11px] text-muted-foreground">自定义唤醒词是实验性的：喊中率和误唤醒还在实测，叫不醒时换回预置的“琴音琴音”。
              <button type="button" class="text-primary hover:underline" onclick={onOpenTalk}>语音唤醒设置 →</button></p>
          </div>
        {/if}
      </SettingsRow>
      <SettingsRow label="角色卡" description="导出成 Character Card V2（.json，别的软件也能读名字和设定）；声音、唤醒词放在 extensions.kotone 里。">
        {#snippet control()}
          <div class="flex gap-2">
            <Button size="sm" variant="outline" disabled={busy} onclick={() => void exportRole()} data-testid="role-export"><Download class="size-3.5" />导出</Button>
            {#if !cur.builtin || cur.id !== DEFAULT_ROLE}
              <Button size="sm" variant="ghost" disabled={busy} onclick={() => void removeRole()} data-testid="role-delete">{cur.builtin ? "恢复默认" : "删除角色"}</Button>
            {/if}
          </div>
        {/snippet}
      </SettingsRow>
    </SettingsSection>

    <SettingsSection title={`性格（${cur.name} 的 SOUL）`} description="她说话的风格和原则，每段对话开始时读一次。也可以直接跟她说“以后说话再简短一点”，她会问你要不要改这里。">
      <div class="space-y-2 py-3.5">
        <textarea class="min-h-56 w-full rounded-lg border border-input bg-background p-3 font-mono text-[13px] leading-relaxed" bind:value={soul}
          aria-label="性格" data-testid="identity-soul-text"></textarea>
        <div class="flex items-center justify-between">
          <span class="text-xs tabular-nums {soulChars > SOUL_LIMIT ? 'text-destructive' : 'text-muted-foreground'}" data-testid="identity-soul-count">{soulChars} / {SOUL_LIMIT} 字</span>
          <Button size="sm" disabled={busy || soul === (cur.soul ?? "") || soulChars > SOUL_LIMIT} onclick={() => void saveSoul()} data-testid="identity-soul-save">保存</Button>
        </div>
      </div>
    </SettingsSection>
  {/if}
</div>
