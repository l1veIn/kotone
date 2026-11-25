<script lang="ts">
  import { onMount } from "svelte";
  import { companionClearChatRegion, companionStatus, type CompanionStatus } from "../../../../lib/ipc";
  import { errText, patchSettings, settingsStore, toast } from "../../../../lib/stores/ui";
  import { m } from "$lib/paraglide/messages.js";
  import { openTarget } from "../../../../lib/stores/navigation";
  import SettingsSection from "$lib/components/settings/SettingsSection.svelte";
  import SettingsRow from "$lib/components/settings/SettingsRow.svelte";
  import { Button } from "$lib/components/ui/button";
  import { Switch } from "$lib/components/ui/switch";
  import { CircleAlert, CircleCheck, Plus } from "@lucide/svelte";

  let status = $state<CompanionStatus | null>(null);
  const config = $derived($settingsStore?.assistant);

  export async function refresh() {
    try { status = await companionStatus(); } catch (error) { toast(false, m.companion_quick_status_load_failed({ error: errText(error) })); }
  }

  onMount(() => { void refresh(); });
  $effect(() => { config?.connectionId; config?.companionBindings?.connectionId; config?.companion; $settingsStore?.ocr?.activeModelId; void refresh(); });
</script>

{#if config}
  <div class="space-y-6" data-testid="companion-settings">
    <SettingsSection title={m.companion_quick_screen_title()} id="companion-title" description={m.companion_quick_screen_desc()}>
      {#if status}
        <div class="py-3.5">
          <p class="m-0 flex items-start gap-2 rounded-lg border px-3 py-2 text-xs leading-relaxed {status.ready ? 'border-success/35 bg-success/8 text-success' : 'border-warning/35 bg-warning/8 text-foreground/85'}">
            {#if status.ready}<CircleCheck class="mt-0.5 size-3.5 shrink-0" />{:else}<CircleAlert class="mt-0.5 size-3.5 shrink-0 text-warning" />{/if}
            <span data-testid="companion-status">{status.ready ? m.companion_quick_status_ready() : status.reason}</span>
          </p>
          {#if status.ocr.state?.dataEgress === "selected-image"}
            <p class="mt-2 mb-0 text-xs leading-relaxed text-brand-pink" data-testid="companion-upload-warning">
              {m.companion_quick_upload_warning()}
            </p>
          {/if}
        </div>
      {/if}
      <SettingsRow label={m.companion_quick_auto_send_label()} description={m.companion_quick_auto_send_desc()}>
        {#snippet control()}
          <Switch aria-label={m.companion_quick_auto_send_label()}
            bind:checked={() => config.companion.autoSend, (checked) => void patchSettings({ assistant: { companion: { autoSend: checked } } }, m.companion_saved())} />
        {/snippet}
      </SettingsRow>
      {#if status?.profile.chatRegion}
        <SettingsRow label={m.companion_quick_chat_region_label()} description={m.companion_quick_chat_region_desc()}>
          {#snippet control()}
            <Button variant="outline" size="sm" onclick={() => companionClearChatRegion().then(refresh).catch((error) => toast(false, errText(error)))}>{m.companion_quick_chat_region_reset()}</Button>
          {/snippet}
        </SettingsRow>
      {/if}
    </SettingsSection>

    <!-- 快捷指令在「设置 → 热键」里加、改（ADR 040 第 3 期 D3、D5）：这里只说一句、给“去加” -->
    <SettingsSection title={m.companion_quick_title()} id="companion-quick-title" description={m.companion_quick_moved()}>
      {#snippet actions()}
        <Button variant="outline" size="sm" data-testid="companion-quick-goto"
          onclick={() => openTarget({ page: "advanced", tab: "hotkeys", anchor: "hotkeys-add-say" })}><Plus class="size-3.5" />{m.companion_quick_goto()}</Button>
      {/snippet}
      {#snippet children()}{/snippet}
    </SettingsSection>
  </div>
{/if}
