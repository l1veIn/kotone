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
  import { m } from "$lib/paraglide/messages.js";
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
    // i18n-ignore-start：角色内容（名字、性格、唤醒词规则，和后端一致），不是界面文字
    if (role.id === DEFAULT_ROLE && role.name === "琴音") return "琴音琴音";
    const chars = [...role.name.trim()];
    return chars.length === 1 ? `小${role.name}小${role.name}` : chars.length === 2 ? role.name + role.name : role.name;
    // i18n-ignore-end
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
  const switchTo = (r: RoleInfo) => run(async () => { await rolesSwitch(r.id); await refresh(r.id); }, m.companion_identity_switched({ name: r.name }));
  const saveName = () => run(async () => {
    if (!role) return;
    const next = name.trim();
    if (!next || [...next].length > NAME_LIMIT) throw new Error(m.companion_identity_name_invalid({ limit: NAME_LIMIT }));
    await rolesUpdate(role.id, { name: next, summary: summary.trim() });
    await refresh(role.id);
  }, m.companion_saved());
  const saveSoul = () => run(async () => {
    if (!role || soulChars > SOUL_LIMIT) return;
    await rolesUpdate(role.id, { soul });
    await refresh(role.id);
  }, m.companion_identity_soul_saved());
  const saveVoice = (voice: string) => run(async () => {
    if (!role) return;
    await rolesUpdate(role.id, { voice });
    await refresh(role.id);
  }, m.companion_identity_voice_saved());

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
    if (wakeText.trim() && !wakeCheck?.ok) throw new Error(wakeCheck?.problems.join("；") || m.companion_identity_wake_unusable());
    await rolesUpdate(role.id, { wake: { text: wakeText.trim(), readings } });
    await refresh(role.id);
  }, m.companion_identity_wake_saved());

  async function pickAvatar(event: Event) {
    const input = event.currentTarget as HTMLInputElement;
    const file = input.files?.[0];
    input.value = "";
    if (!file || !role) return;
    avatarError = "";
    if (file.size > 2 * 1024 * 1024) { avatarError = m.companion_identity_avatar_too_large(); return; }
    const ext = file.name.split(".").pop() ?? "";
    const id = role.id;
    await run(async () => {
      const saved = await workspaceSetAvatar(new Uint8Array(await file.arrayBuffer()), ext, id);
      await rolesUpdate(id, { avatar: saved });
      await refresh(id);
    }, m.companion_identity_avatar_updated()).catch((e) => { avatarError = errText(e); });
  }
  const clearAvatar = () => run(async () => { if (role) { await rolesUpdate(role.id, { avatar: "" }); await refresh(role.id); } }, m.companion_identity_avatar_cleared());

  const createRole = () => run(async () => {
    const r = await rolesCreate({ name: newName.trim() });
    newName = "";
    await refresh(r.id);
  }, m.companion_identity_role_created());
  const removeRole = () => run(async () => {
    if (!role) return;
    const id = role.id;
    await rolesDelete(id);
    await refresh(role.builtin ? id : undefined);
  }, role?.builtin ? m.companion_identity_role_restored() : m.companion_identity_role_deleted());
  const exportRole = () => run(async () => {
    if (!role) return;
    const card = await rolesExport(role.id);
    const url = URL.createObjectURL(new Blob([JSON.stringify(card, null, 2)], { type: "application/json" }));
    const a = document.createElement("a");
    a.href = url;
    a.download = `${role.name}.card.json`;
    a.click();
    setTimeout(() => URL.revokeObjectURL(url), 1000);
  }, m.companion_identity_card_exported());
  async function importCard(event: Event) {
    const input = event.currentTarget as HTMLInputElement;
    const file = input.files?.[0];
    input.value = "";
    if (!file) return;
    await run(async () => {
      const card = JSON.parse(await file.text());
      const r = await rolesImport(card);
      await refresh(r.id);
    }, m.companion_identity_card_imported());
  }

  const voiceOptions = $derived([{ value: "", label: m.companion_identity_voice_follow() }, ...voices.map((v) => ({ value: v.id, label: v.name }))]);
</script>

<div class="space-y-6" data-testid="identity-tab">
  <SettingsSection title={m.companion_identity_title()} description={m.companion_identity_desc()}>
    {#if error}<p role="alert" class="m-0 text-xs text-destructive">{error}</p>{/if}
    <div class="grid gap-2 py-3 sm:grid-cols-2 lg:grid-cols-3" data-testid="role-list">
      {#each roles as r (r.id)}
        <div role="button" tabindex="0" data-testid="role-card" data-role={r.id} aria-pressed={r.id === selectedId}
          class="flex cursor-pointer items-center gap-3 rounded-xl border p-3 text-left transition-colors {r.id === selectedId ? 'border-primary/60 bg-primary/5' : 'border-border/80 hover:bg-muted/40'}"
          onclick={() => select(r.id)} onkeydown={(e) => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); select(r.id); } }}>
          <KotoneAvatar size={40} mood="asking" custom={avatars[r.id] ?? null} faces={bundledFaces(r.id, r.avatar)} letter={placeholderLetter(r.id, r.name, r.avatar)} />
          <div class="min-w-0 flex-1">
            <div class="flex min-w-0 items-center gap-1.5 text-sm font-medium"><span class="truncate">{r.name}</span>{#if r.current}<span class="shrink-0 rounded-full bg-primary/15 px-1.5 text-[11px] text-primary" data-testid="role-current">{m.companion_identity_current()}</span>{/if}</div>
            <p class="m-0 truncate text-xs text-muted-foreground">{r.summary || m.companion_identity_custom_role()}</p>
          </div>
          {#if !r.current}
            <Button size="sm" variant="outline" class="shrink-0" disabled={busy} aria-label={m.companion_identity_switch_to({ name: r.name })} title={m.companion_identity_switch_to({ name: r.name })} onclick={(e) => { e.stopPropagation(); void switchTo(r); }} data-testid="role-switch">{m.companion_identity_switch()}</Button>
          {/if}
        </div>
      {/each}
    </div>
    <div class="flex flex-wrap items-center gap-2 pb-2">
      <Input class="h-8 w-40" bind:value={newName} maxlength={NAME_LIMIT} placeholder={m.companion_identity_new_name_placeholder()} data-testid="role-new-name"
        onkeydown={(e) => { if (e.key === "Enter" && newName.trim()) void createRole(); }} />
      <Button size="sm" disabled={busy || !newName.trim()} onclick={() => void createRole()} data-testid="role-create"><Plus class="size-3.5" />{m.companion_identity_create()}</Button>
      <input bind:this={cardInput} type="file" accept=".json,application/json" class="hidden" onchange={(e) => void importCard(e)} data-testid="role-import-input" />
      <Button size="sm" variant="ghost" disabled={busy} onclick={() => cardInput?.click()}><Upload class="size-3.5" />{m.companion_identity_import_card()}</Button>
    </div>
  </SettingsSection>

  {#if role}
    <SettingsSection title={m.companion_identity_edit_title({ name: cur.name })} description={cur.builtin ? m.companion_identity_builtin_desc() : m.companion_identity_custom_desc()}>
      <SettingsRow label={m.companion_identity_name_label()} for="role-name" description={m.companion_identity_name_desc({ limit: NAME_LIMIT })}>
        {#snippet control()}
          <div class="flex flex-wrap items-center gap-2">
            <Input id="role-name" class="h-8 w-36" bind:value={name} maxlength={NAME_LIMIT} data-testid="identity-name"
              onkeydown={(e) => { if (e.key === "Enter") void saveName(); }} />
            <Input class="h-8 w-48" bind:value={summary} maxlength={60} placeholder={m.companion_identity_summary_placeholder()} data-testid="role-summary" />
            <Button size="sm" disabled={busy || (name.trim() === cur.name && summary.trim() === cur.summary)} onclick={() => void saveName()} data-testid="identity-name-save">{m.companion_save()}</Button>
          </div>
        {/snippet}
      </SettingsRow>
      <SettingsRow label={m.companion_identity_avatar_label()} description={cur.id === DEFAULT_ROLE || bundledFaces(cur.id, "") ? m.companion_identity_avatar_desc_faces() : m.companion_identity_avatar_desc_letter()}>
        {#snippet control()}
          <div class="flex items-center gap-3">
            <div data-testid="identity-avatar" data-src={avatars[cur.id] ?? ""}><KotoneAvatar size={48} mood="asking" custom={avatars[cur.id] ?? null} faces={bundledFaces(cur.id, cur.avatar)} letter={placeholderLetter(cur.id, cur.name, cur.avatar)} /></div>
            <input bind:this={fileInput} type="file" accept=".png,.jpg,.jpeg,.webp,.gif,image/png,image/jpeg,image/webp,image/gif" class="hidden"
              onchange={(e) => void pickAvatar(e)} data-testid="identity-avatar-input" />
            <Button size="sm" variant="outline" disabled={busy} onclick={() => fileInput?.click()}>{m.companion_identity_avatar_change()}</Button>
            {#if cur.avatar}<Button size="sm" variant="ghost" disabled={busy} onclick={() => void clearAvatar()}>{m.companion_identity_avatar_default()}</Button>{/if}
          </div>
        {/snippet}
        {#if avatarError}<p role="alert" class="m-0 text-xs text-destructive" data-testid="identity-avatar-error">{avatarError}</p>{/if}
        {#if FACE_CREDITS[cur.id] && !cur.avatar}<p class="m-0 text-[11px] leading-relaxed text-muted-foreground" data-testid="role-face-credit">{FACE_CREDITS[cur.id]}</p>{/if}
      </SettingsRow>
      <SettingsRow label={m.companion_identity_voice_label()} description={m.companion_identity_voice_desc()}>
        {#snippet control()}
          <div class="w-60 max-w-full"><SelectField ariaLabel={m.companion_identity_voice_aria()} value={cur.voice} options={voiceOptions.some((o) => o.value === cur.voice) ? voiceOptions : [...voiceOptions, { value: cur.voice, label: cur.voice }]}
            disabled={busy} onchange={(v) => void saveVoice(v)} testid="role-voice" /></div>
        {/snippet}
      </SettingsRow>
      <SettingsRow label={m.companion_identity_wake_label()} for="role-wake" description={m.companion_identity_wake_desc({ wake: defaultWake })}>
        {#snippet control()}
          <div class="flex items-center gap-2">
            <Input id="role-wake" class="h-8 w-40" bind:value={wakeText} maxlength={8} placeholder={defaultWake} data-testid="role-wake"
              oninput={() => { readings = []; recheck(); }} />
            <Button size="sm" disabled={busy || (wakeText.trim() === cur.wake.text && readings.join() === cur.wake.readings.join())} onclick={() => void saveWake()} data-testid="role-wake-save">{m.companion_save()}</Button>
          </div>
        {/snippet}
        {#if wakeCheck}
          <div class="mt-1 space-y-1" data-testid="role-wake-check" data-ok={wakeCheck.ok}>
            <div class="flex flex-wrap gap-1.5">
              {#each wakeCheck.syllables as s, i (i)}
                <span class="inline-flex flex-col items-center rounded-md border px-1.5 py-0.5 text-xs {s.missing.length ? 'border-destructive/60 text-destructive' : 'border-border/80'}">
                  <span class="text-sm">{s.character}</span>
                  {#if s.choices.length > 1}
                    <select class="bg-transparent text-[11px] text-primary" aria-label={m.companion_identity_wake_reading_aria({ character: s.character })} value={s.reading}
                      onchange={(e) => pickReading(i, e.currentTarget.value)} data-testid="role-wake-reading">
                      {#each s.choices as c}<option value={c}>{c}</option>{/each}
                    </select>
                  {:else}<span class="text-[11px] text-muted-foreground">{s.reading}</span>{/if}
                </span>
              {/each}
            </div>
            {#if wakeCheck.ok}<p class="m-0 flex items-center gap-1 text-xs text-success"><Check class="size-3" />{m.companion_identity_wake_ok()}{#if wakeCheck.warnings.length}<span class="text-warning">{m.companion_identity_wake_warnings({ warnings: wakeCheck.warnings.join("；") })}</span>{/if}</p>
            {:else}<p class="m-0 text-xs text-destructive" role="alert" data-testid="role-wake-problem">{wakeCheck.problems.join("；")}</p>{/if}
            <p class="m-0 text-[11px] text-muted-foreground">{m.companion_identity_wake_experimental()}
              <button type="button" class="text-primary hover:underline" onclick={onOpenTalk}>{m.companion_identity_wake_settings_link()}</button></p>
          </div>
        {/if}
      </SettingsRow>
      <SettingsRow label={m.companion_identity_card_label()} description={m.companion_identity_card_desc()}>
        {#snippet control()}
          <div class="flex gap-2">
            <Button size="sm" variant="outline" disabled={busy} onclick={() => void exportRole()} data-testid="role-export"><Download class="size-3.5" />{m.companion_identity_export()}</Button>
            {#if !cur.builtin || cur.id !== DEFAULT_ROLE}
              <Button size="sm" variant="ghost" disabled={busy} onclick={() => void removeRole()} data-testid="role-delete">{cur.builtin ? m.companion_identity_restore_default() : m.companion_identity_delete_role()}</Button>
            {/if}
          </div>
        {/snippet}
      </SettingsRow>
    </SettingsSection>

    <SettingsSection title={m.companion_identity_soul_title({ name: cur.name })} description={m.companion_identity_soul_desc()}>
      <div class="space-y-2 py-3.5">
        <textarea class="min-h-56 w-full rounded-lg border border-input bg-background p-3 font-mono text-[13px] leading-relaxed" bind:value={soul}
          aria-label={m.companion_identity_soul_aria()} data-testid="identity-soul-text"></textarea>
        <div class="flex items-center justify-between">
          <span class="text-xs tabular-nums {soulChars > SOUL_LIMIT ? 'text-destructive' : 'text-muted-foreground'}" data-testid="identity-soul-count">{m.companion_char_count({ count: soulChars, limit: SOUL_LIMIT })}</span>
          <Button size="sm" disabled={busy || soul === (cur.soul ?? "") || soulChars > SOUL_LIMIT} onclick={() => void saveSoul()} data-testid="identity-soul-save">{m.companion_save()}</Button>
        </div>
      </div>
    </SettingsSection>
  {/if}
</div>
