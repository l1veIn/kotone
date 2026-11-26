<script lang="ts">
  // 插件的内置推送（ADR 043 D4）：插件页「设置」里宿主画的一节“推送”（插件自己的 App 和设置页不用画）。照清单 `[[pushes]]`
  // 每条一行：名字、样子（卡片 / 一行 / 语音）、时机、做什么，一个开关。开关在 `plugins.<id>.pushes.<推送>`，由宿主执行
  // （关掉的推送宿主直接丢掉，开关一定管用）；内容不能改——想换说法或时机，玩家自己建自动化（自动化能订阅同样的事件）。
  // 照 Android 的通知渠道：App 在清单里声明会发哪几类，系统给每类画一个开关、由系统执行。
  import { Switch } from "$lib/components/ui/switch";
  import SettingsSection from "$lib/components/settings/SettingsSection.svelte";
  import { companionName } from "$lib/stores/companion";
  import { errText, settingsStore, toast } from "$lib/stores/ui";
  import {
    pluginEnabled, pushSwitch, pushTitle, pushWhat, pushWhen, setPluginPush, sourceEnabled, sourceName,
    type PluginInfo, type PushForm,
  } from "$lib/plugins";
  import { m } from "$lib/paraglide/messages.js";

  let { plugin }: { plugin: PluginInfo } = $props();
  const manifest = $derived(plugin.manifest);
  const pushes = $derived(manifest.pushes ?? []);
  const on = $derived(pluginEnabled($settingsStore, manifest));
  const FORMS: Record<PushForm, () => string> = {
    card: m.system_plugin_push_form_card,
    line: m.system_plugin_push_form_line,
    voice: m.system_plugin_push_form_voice,
  };
  /** 这条推送要的、现在关着的数据开关的名字（插件关着时不写：整节都灰着）。 */
  function needsOff(needs: string[]): string {
    return needs
      .filter((id) => !sourceEnabled($settingsStore, manifest, id))
      .map((id) => { const source = manifest.sources.find((s) => s.id === id); return source ? sourceName(manifest, source) : id; })
      .join(m.hotkeys_list_separator());
  }

  async function save(id: string, title: string, enabled: boolean) {
    try {
      await setPluginPush(manifest.id, id, enabled);
      toast(true, enabled ? m.system_plugin_push_on({ title }) : m.system_plugin_push_off({ title }));
    } catch (error) {
      toast(false, m.system_plugins_save_failed({ error: errText(error) }));
    }
  }
</script>

{#if pushes.length}
  <SettingsSection title={m.system_plugin_pushes_title()} description={m.system_plugin_pushes_desc({ companion: $companionName })}
    testid={`plugin-${manifest.id}-pushes`}>
    {#each pushes as push (push.id)}
      {@const title = pushTitle(manifest.id, push)}
      {@const off = on ? needsOff(push.needs) : ""}
      <div class="flex items-start justify-between gap-4 py-3" data-testid={`plugin-${manifest.id}-push-${push.id}-row`}>
        <div class="min-w-0 space-y-1">
          <div class="flex flex-wrap items-center gap-1.5">
            <label for={`push-${manifest.id}-${push.id}`} class="cursor-pointer text-[13px] font-medium text-foreground">{title}</label>
            {#each push.forms as form (form)}
              <span class="rounded-md bg-muted px-1.5 py-px text-[11px] text-muted-foreground" data-testid="plugin-push-form">{FORMS[form]()}</span>
            {/each}
          </div>
          <p class="m-0 text-xs leading-relaxed text-muted-foreground" data-testid="plugin-push-when">{m.system_plugin_push_when({ when: pushWhen(manifest, push) })}</p>
          <p class="m-0 text-xs leading-relaxed text-muted-foreground" data-testid="plugin-push-what">{m.system_plugin_push_what({ what: pushWhat(manifest, push) })}</p>
          {#if off}
            <p class="m-0 text-xs leading-relaxed text-warning" data-testid="plugin-push-needs-off">{m.system_plugin_push_needs_off({ sources: off })}</p>
          {/if}
        </div>
        <!-- 受控：显示的总是设置里的值，保存失败时不会停在错误的位置 -->
        <Switch id={`push-${manifest.id}-${push.id}`} class="mt-0.5" disabled={!on} aria-label={title}
          data-testid={`plugin-${manifest.id}-push-${push.id}`}
          bind:checked={() => pushSwitch($settingsStore, manifest, push), (next) => void save(push.id, title, next)} />
      </div>
    {/each}
  </SettingsSection>
{/if}
