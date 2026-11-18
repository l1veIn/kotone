<script lang="ts">
  // 「琴音 → 她是谁」（ADR 022 / 023）：名字、头像、性格（SOUL.md）。
  // 名字与头像存在设置 `agent.identity`（气泡、聊天、系统提示都用它）；SOUL 是工作区文件。
  import { onMount } from "svelte";
  import { Button } from "$lib/components/ui/button";
  import { Input } from "$lib/components/ui/input";
  import SettingsSection from "$lib/components/settings/SettingsSection.svelte";
  import SettingsRow from "$lib/components/settings/SettingsRow.svelte";
  import WorkspaceFileEditor from "./WorkspaceFileEditor.svelte";
  import { workspaceAvatar, workspaceOverview, workspaceSetAvatar, type WorkspaceOverview } from "../../../../lib/workspace";
  import { errText, patchSettings, settingsStore } from "../../../../lib/stores/ui";
  import cutout from "../../../../assets/brand/kotone-cutout.webp";

  let { onOpenTalk }: { onOpenTalk: () => void } = $props();

  let overview = $state<WorkspaceOverview | null>(null);
  let name = $state("");
  let avatarUrl = $state<string | null>(null);
  let avatarError = $state("");
  let fileInput = $state<HTMLInputElement>();
  const savedName = $derived($settingsStore?.agent?.identity?.name ?? "");
  const avatarFile = $derived($settingsStore?.agent?.identity?.avatar ?? "");
  const NAME_LIMIT = 20;

  onMount(() => {
    name = savedName;
    void workspaceOverview().then((value) => { overview = value; }).catch(() => {});
  });

  // 头像文件变了就重新读。
  $effect(() => {
    const file = avatarFile;
    let url: string | null = null;
    let disposed = false;
    void workspaceAvatar(file).then((bytes) => {
      if (disposed || !bytes) { if (!disposed) avatarUrl = null; return; }
      url = URL.createObjectURL(new Blob([bytes as BlobPart]));
      avatarUrl = url;
    }).catch(() => { if (!disposed) avatarUrl = null; });
    return () => { disposed = true; if (url) URL.revokeObjectURL(url); };
  });

  async function saveName() {
    const next = name.trim();
    if ([...next].length > NAME_LIMIT) return;
    await patchSettings({ agent: { identity: { name: next } } }, next ? `她现在叫「${next}」` : "名字已恢复为「琴音」");
  }

  async function pickAvatar(event: Event) {
    const input = event.currentTarget as HTMLInputElement;
    const file = input.files?.[0];
    input.value = "";
    if (!file) return;
    avatarError = "";
    if (file.size > 2 * 1024 * 1024) { avatarError = "头像不能超过 2 MB"; return; }
    const ext = file.name.split(".").pop() ?? "";
    try {
      const saved = await workspaceSetAvatar(new Uint8Array(await file.arrayBuffer()), ext);
      await patchSettings({ agent: { identity: { avatar: saved } } }, "头像已更新");
    } catch (e) {
      avatarError = errText(e);
    }
  }
</script>

<div class="space-y-6" data-testid="identity-tab">
  <SettingsSection title="名字与头像" description="气泡、聊天里显示的名字和形象；她也会用这个名字称呼自己。">
    <SettingsRow label="名字" for="companion-name" description={`空着就叫「${overview?.defaultName ?? "琴音"}」，最多 ${NAME_LIMIT} 个字`}>
      {#snippet control()}
        <div class="flex items-center gap-2">
          <Input id="companion-name" class="h-8 w-44" bind:value={name} maxlength={NAME_LIMIT} placeholder="琴音" data-testid="identity-name"
            onkeydown={(e) => { if (e.key === "Enter") void saveName(); }} />
          <Button size="sm" disabled={name.trim() === savedName} onclick={() => void saveName()} data-testid="identity-name-save">保存</Button>
        </div>
      {/snippet}
    </SettingsRow>
    <SettingsRow label="头像" description="PNG、JPG、WebP 或 GIF，不超过 2 MB">
      {#snippet control()}
        <div class="flex items-center gap-3">
          <img src={avatarUrl ?? cutout} alt="头像" class="size-12 rounded-full border border-hairline bg-muted object-cover" data-testid="identity-avatar" />
          <input bind:this={fileInput} type="file" accept=".png,.jpg,.jpeg,.webp,.gif,image/png,image/jpeg,image/webp,image/gif" class="hidden"
            onchange={(e) => void pickAvatar(e)} data-testid="identity-avatar-input" />
          <Button size="sm" variant="outline" onclick={() => fileInput?.click()}>换一张</Button>
          {#if avatarFile}<Button size="sm" variant="ghost" onclick={() => void patchSettings({ agent: { identity: { avatar: "" } } }, "已恢复默认形象")}>用默认</Button>{/if}
        </div>
      {/snippet}
      {#if avatarError}<p role="alert" class="m-0 text-xs text-destructive" data-testid="identity-avatar-error">{avatarError}</p>{/if}
    </SettingsRow>
    <SettingsRow label="怎么叫她" description="呼叫热键、唤醒词在「设置 → 呼叫」里">
      {#snippet control()}<Button size="sm" variant="outline" onclick={onOpenTalk}>去设置</Button>{/snippet}
    </SettingsRow>
  </SettingsSection>

  <SettingsSection title="性格（SOUL）" description="她说话的风格和原则，每段对话开始时读一次。也可以直接跟她说“以后说话再简短一点”，她会问你要不要改这里。">
    <div class="py-3.5">
      <WorkspaceFileEditor file="SOUL.md" limit={overview?.files.soul.limit ?? 4000} label="性格" testid="identity-soul" />
    </div>
  </SettingsSection>
</div>
