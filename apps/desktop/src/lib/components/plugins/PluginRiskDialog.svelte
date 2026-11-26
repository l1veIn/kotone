<script lang="ts">
  // 一个插件的“风险和许可”（ADR 043 D8）：风险在前（每个有风险的数据开关：名字、现在开没开、风险提示；文字里的
  // github.com/… 这类地址能点），许可和出处在后（清单的 `[[notices]]`）。只是提示，不拦着（原则 3）：关掉这个窗口
  // 不改任何开关，开关照玩家选的。
  // 什么时候弹由用它的地方定：插件页（PluginPage）第一次看到这个插件有风险的开关开着时（默认就开着的、刚打开的），
  // 游戏配置页里英雄联盟那一节刚打开“读客户端数据”时；以后从插件「设置」里的“风险和许可”打开。
  // 弹出来就记下看过（`plugins.<id>.riskSeen`），同一版不再自己弹。
  import { TriangleAlert } from "@lucide/svelte";
  import * as Dialog from "$lib/components/ui/dialog";
  import { Button } from "$lib/components/ui/button";
  import { settingsStore } from "$lib/stores/ui";
  import {
    markRiskSeen, noticeBody, noticeTitle, pluginName, riskUnseen, sourceEnabled, sourceName, sourceRisk, type PluginInfo,
  } from "$lib/plugins";
  import { openExternal } from "$lib/ipc";
  import { m } from "$lib/paraglide/messages.js";

  let { plugin, open = $bindable(false) }: { plugin: PluginInfo; open?: boolean } = $props();

  const manifest = $derived(plugin.manifest);
  const id = $derived(manifest.id);
  const risks = $derived(manifest.sources.filter((s) => s.risk));
  const notices = $derived(manifest.notices ?? []);

  /** 文字里的网址（`github.com/LeagueAkari/LeagueAkari/issues/523`、`developer.riotgames.com/docs/lol`）：小写域名 + 路径。 */
  const LINK = /((?:https:\/\/)?(?:[a-z0-9-]+\.)+[a-z]{2,}\/[\w.\-/#%?=&]*[\w/])/;
  /** 拆成字和链接（split 带一个捕获组：奇数位是链接）。 */
  function segments(text: string): { text: string; link: boolean }[] {
    return text.split(LINK).map((part, i) => ({ text: part, link: i % 2 === 1 })).filter((part) => part.text);
  }
  const href = (link: string) => (link.startsWith("https://") ? link : `https://${link}`);
  /** 链接显示成什么：去掉 https:// 和末尾的斜杠。 */
  const shown = (link: string) => link.replace(/^https:\/\//, "").replace(/\/$/, "");

  // 弹出来就记下看过（保存失败只是下次再弹一次）
  let marking = false;
  $effect(() => {
    if (!open || marking || !riskUnseen($settingsStore, manifest)) return;
    marking = true;
    markRiskSeen(id)
      .catch((error) => console.warn("风险和许可没记上", error))
      .finally(() => (marking = false));
  });
</script>

{#snippet linked(text: string, testid: string)}
  {#each segments(text) as part, i (i)}{#if part.link}<button type="button" class="underline underline-offset-2 hover:text-foreground"
    data-testid={testid} onclick={() => void openExternal(href(part.text))}>{part.text}</button>{:else}{part.text}{/if}{/each}
{/snippet}

<Dialog.Root bind:open>
  <Dialog.Content class="ui-scope max-h-[85vh] overflow-y-auto sm:max-w-lg" data-testid={`plugin-risk-dialog-${id}`}>
    <Dialog.Header>
      <Dialog.Title>{m.system_plugins_risk_title({ name: pluginName(manifest) })}</Dialog.Title>
      <Dialog.Description>{m.system_plugins_risk_intro()}</Dialog.Description>
    </Dialog.Header>

    {#if risks.length}
      <section class="space-y-2" data-testid="plugin-risk-risks">
        <h3 class="m-0 flex items-center gap-1.5 text-xs font-semibold text-foreground">
          <TriangleAlert class="size-3.5 text-warning" />{m.system_plugins_risk_section()}
        </h3>
        <ul class="m-0 list-none space-y-2.5 p-0">
          {#each risks as source (source.id)}
            {@const on = sourceEnabled($settingsStore, manifest, source.id)}
            <li class="space-y-1" data-testid={`plugin-risk-${id}-${source.id}`} data-on={on}>
              <p class="m-0 flex items-center gap-2 text-[13px] font-medium text-foreground">
                {sourceName(manifest, source)}
                <span class="rounded px-1.5 text-[10px] font-normal {on ? 'bg-warning/15 text-warning' : 'bg-muted text-muted-foreground'}">
                  {on ? m.system_plugins_risk_on() : m.system_plugins_risk_off()}</span>
              </p>
              <p class="m-0 text-xs leading-relaxed text-muted-foreground">{@render linked(sourceRisk(manifest, source), `plugin-risk-${id}-${source.id}-link`)}</p>
            </li>
          {/each}
        </ul>
      </section>
    {/if}

    {#if notices.length}
      <section class="space-y-2 border-t border-hairline pt-3" data-testid="plugin-risk-notices">
        <h3 class="m-0 text-xs font-semibold text-foreground">{m.system_plugins_notice_section()}</h3>
        <ul class="m-0 list-none space-y-2.5 p-0">
          {#each notices as notice (notice.id)}
            <li class="space-y-1" data-testid={`plugin-notice-${id}-${notice.id}`}>
              <p class="m-0 text-[13px] font-medium text-foreground">{noticeTitle(manifest, notice)}</p>
              <p class="m-0 text-xs leading-relaxed text-muted-foreground">{@render linked(noticeBody(manifest, notice), `plugin-notice-${id}-${notice.id}-body-link`)}</p>
              {#if notice.links.length}
                <p class="m-0 flex flex-wrap gap-x-3 gap-y-0.5 text-[11px] text-muted-foreground">
                  {#each notice.links as link (link)}
                    <button type="button" class="break-all underline underline-offset-2 hover:text-foreground"
                      data-testid={`plugin-notice-${id}-${notice.id}-link`} onclick={() => void openExternal(link)}>{shown(link)}</button>
                  {/each}
                </p>
              {/if}
            </li>
          {/each}
        </ul>
      </section>
    {/if}

    <Dialog.Footer class="items-center gap-3 sm:justify-between">
      <p class="m-0 text-[11px] leading-relaxed text-muted-foreground">{m.system_plugins_risk_later()}</p>
      <Button size="sm" data-testid="plugin-risk-ok" onclick={() => (open = false)}>{m.system_plugins_risk_ok()}</Button>
    </Dialog.Footer>
  </Dialog.Content>
</Dialog.Root>
