<script lang="ts" module>
  export type { PageId } from "./sidebarItems";
</script>

<script lang="ts">
  /*
   * 设置窗口侧边栏（ADR 028，参照 DeepSeek Harness 的侧边栏：顶部“新对话”和会话列表、底部固定的设置项）。
   * 新对话和对话列表在最上面并占满剩余高度；「游戏陪玩」分组固定在底部，始终看得见。
   * 语音输入搬进了插件（ADR 038）：从「插件 → 语音输入」打开，左侧栏没有单独一项。
   * 三种形态：展开（236px）、图标栏（56px）、窄窗口时的抽屉。
   * 两组里的页面可以自己排（ADR 034 第六节）：右键上移 / 下移 / 隐藏，“⋯”打开全部页面（对话框在 Settings.svelte）。
   * 在放歌时底部有一个小播放条（ADR 034）。插件（ADR 038，2026-11-24 玩家定的）：左侧栏只有一项「插件」，
   * 点开是插件的卡片；打开某个插件的 App 时「插件」亮着。
   */
  import { companionName } from "$lib/stores/companion";
  import { ArrowDown, ArrowUp, Ellipsis, EyeOff, LayoutList, MessageSquarePlus } from "@lucide/svelte";
  import { ContextMenu } from "bits-ui";
  import ConversationList from "../../lib/agentChat/ConversationList.svelte";
  import { useAgentChat } from "../../lib/agentChat/context";
  import { toast, errText, settingsStore } from "../../lib/stores/ui";
  import { hasLegacyReminders } from "../../lib/ipc";
  import { sidebarPagesOpen } from "../../lib/stores/navigation";
  import { arrange, canMove, movePage, normalizePrefs, setPageHidden } from "../../lib/sidebarLayout";
  import { footItems as fixedItems, groupLabel, navItems, presentGroups, saveSidebarPrefs, type NavItem, type PageId } from "./sidebarItems";
  import { pluginOfPage, pluginPageId, pluginsStore } from "../../lib/plugins";
  import { enabledSlots } from "../../lib/pluginUi";
  import iconSrc from "../../assets/brand/icon-src.webp";
  import { m } from "$lib/paraglide/messages.js";

  let { page, chatActive = false, rail = false, legacyVoiceOnly = false, onNavigate, onOpenChat, onOpenRole }: {
    page: PageId;
    /** 当前在聊天（而不是角色的 角色 / 记忆 / 技能 分页）。 */
    chatActive?: boolean;
    legacyVoiceOnly?: boolean;
    /** 收成图标栏。 */
    rail?: boolean;
    onNavigate: (id: PageId) => void;
    /** 打开聊天（新对话、点了某段对话之后）。 */
    onOpenChat: () => void;
    /** 打开角色页（角色 · 记忆 · 技能）。 */
    onOpenRole: () => void;
  } = $props();

  const chat = useAgentChat();
  type Item = NavItem;
  // 分组名“游戏陪玩”由玩家确认（ADR 028 D1）。「琴音」一项用她现在的名字（ADR 022：可以改名）。
  // 组内顺序和隐藏的页面按设置 `ui.sidebar`；整组都藏了就不显示组名。
  const prefs = $derived(normalizePrefs($settingsStore?.ui?.sidebar));
  const items = $derived(navItems($companionName, legacyVoiceOnly));
  const present = $derived(presentGroups(hasLegacyReminders($settingsStore?.ui)));
  const pageLists = $derived(present.map((g) => g.pages));
  const groups: { label: string; items: Item[] }[] = $derived(
    present
      .map((group) => ({
        label: groupLabel(group.id),
        items: arrange(group.pages, prefs).filter((id) => !prefs.hidden.includes(id)).map((id) => items[id]).filter((item): item is Item => !!item),
      }))
      .filter((group) => group.items.length > 0),
  );
  const hiddenCount = $derived(pageLists.flat().filter((id) => prefs.hidden.includes(id)).length);
  const footItems: Item[] = fixedItems();
  const menuItem = "flex cursor-default items-center gap-2 rounded-md px-2 py-1.5 text-[13px] outline-none select-none data-highlighted:bg-accent data-highlighted:text-accent-foreground data-disabled:pointer-events-none data-disabled:opacity-45";

  function isActive(id: PageId): boolean {
    if (id === "assistant") return page === "assistant" && !chatActive;
    if (id === "about") return page === "about" || page === "character";
    // 插件的 App 从「插件」的卡片点进来
    if (id === "plugins") return page === "plugins" || pluginOfPage(page) !== null;
    return page === id;
  }

  async function guarded(action: () => Promise<void>, failure: (error: string) => string) {
    try { await action(); onOpenChat(); }
    catch (error) { toast(false, failure(errText(error))); }
  }
  function newChat() {
    if (legacyVoiceOnly || chat.currentId === null || chat.running) { onOpenChat(); return; }
    void guarded(() => chat.newConversation(), (error) => m.shell_sidebar_new_chat_failed({ error }));
  }
</script>

{#snippet navButton(item: Item, props: Record<string, unknown> = {})}
  {@const active = isActive(item.id)}
  {@const Icon = item.icon}
  <button
    {...props}
    type="button"
    data-testid={`settings-nav-${item.id}`}
    aria-current={active ? "page" : undefined}
    aria-label={rail ? item.label : undefined}
    title={rail ? item.label : undefined}
    class="group relative flex h-8 w-full items-center gap-2.5 rounded-lg text-left text-[13px] transition-colors
      {rail ? 'justify-center' : 'px-2.5'}
      {active ? 'bg-sidebar-accent font-medium text-sidebar-accent-foreground' : 'text-sidebar-foreground hover:bg-sidebar-accent/60 hover:text-foreground'}"
    onclick={() => (item.id === "assistant" ? onOpenRole() : onNavigate(item.id))}
  >
    {#if active}<span class="absolute top-1/2 left-0 h-4 w-[3px] -translate-y-1/2 rounded-full bg-primary shadow-[0_0_8px_var(--color-primary)]"></span>{/if}
    <Icon class="size-4 shrink-0 {active ? 'text-primary' : 'text-sidebar-foreground/75 group-hover:text-foreground'}" />
    {#if !rail}<span class="truncate">{item.label}</span>{/if}
  </button>
{/snippet}

{#snippet navItem(item: Item)}{@render navButton(item)}{/snippet}

<!-- 两组里的页面：右键 上移 / 下移 / 隐藏 / 全部页面（ADR 034 第六节） -->
{#snippet editableItem(item: Item)}
  <ContextMenu.Root>
    <ContextMenu.Trigger>
      {#snippet child({ props })}{@render navButton(item, props)}{/snippet}
    </ContextMenu.Trigger>
    <ContextMenu.Portal>
      <ContextMenu.Content class="ui-scope z-[130] min-w-40 rounded-lg bg-popover p-1 text-popover-foreground shadow-md ring-1 ring-foreground/10" data-testid="sidebar-context-menu">
        <ContextMenu.Item class={menuItem} disabled={!canMove(pageLists, prefs, item.id, -1)} data-testid="sidebar-menu-up"
          onSelect={() => void saveSidebarPrefs(movePage(pageLists, prefs, item.id, -1))}><ArrowUp class="size-3.5" />{m.shell_sidebar_menu_up()}</ContextMenu.Item>
        <ContextMenu.Item class={menuItem} disabled={!canMove(pageLists, prefs, item.id, 1)} data-testid="sidebar-menu-down"
          onSelect={() => void saveSidebarPrefs(movePage(pageLists, prefs, item.id, 1))}><ArrowDown class="size-3.5" />{m.shell_sidebar_menu_down()}</ContextMenu.Item>
        <ContextMenu.Item class={menuItem} data-testid="sidebar-menu-hide"
          onSelect={() => void saveSidebarPrefs(setPageHidden(prefs, item.id, true))}><EyeOff class="size-3.5" />{m.shell_sidebar_menu_hide()}</ContextMenu.Item>
        <ContextMenu.Separator class="mx-1 my-1 h-px bg-hairline" />
        <ContextMenu.Item class={menuItem} data-testid="sidebar-menu-all" onSelect={() => sidebarPagesOpen.set(true)}><LayoutList class="size-3.5" />{m.shell_sidebar_all_pages_menu()}</ContextMenu.Item>
      </ContextMenu.Content>
    </ContextMenu.Portal>
  </ContextMenu.Root>
{/snippet}

{#snippet allPagesButton()}
  <button type="button" data-testid="sidebar-all-pages" onclick={() => sidebarPagesOpen.set(true)}
    aria-label={hiddenCount ? m.shell_sidebar_all_pages_hidden({ count: hiddenCount }) : m.shell_sidebar_all_pages()}
    title={hiddenCount ? m.shell_sidebar_all_pages_hidden({ count: hiddenCount }) : m.shell_sidebar_all_pages()}
    class="flex shrink-0 items-center gap-1 rounded-md text-sidebar-foreground/55 transition-colors hover:bg-sidebar-accent/60 hover:text-foreground
      {rail ? 'h-8 w-full justify-center' : 'h-5 px-1'}">
    <Ellipsis class="size-4" />
    {#if hiddenCount && !rail}<span class="text-[10px] tabular-nums" data-testid="sidebar-hidden-count">{hiddenCount}</span>{/if}
  </button>
{/snippet}

<aside
  class="flex h-full shrink-0 flex-col bg-sidebar pt-1 pb-2 text-sidebar-foreground {rail ? 'w-14 px-2' : 'w-59 px-2'}"
  data-testid="app-sidebar" data-rail={rail}
>
  <button type="button" data-testid="sidebar-new-chat" onclick={newChat}
    aria-label={rail ? m.shell_sidebar_new_chat() : undefined} title={rail ? m.shell_sidebar_new_chat() : undefined}
    class="flex h-9 w-full shrink-0 items-center gap-2.5 rounded-lg border border-hairline text-[13px] text-foreground transition-colors hover:border-primary/40 hover:bg-sidebar-accent/60
      {rail ? 'justify-center' : 'px-2.5'}">
    <MessageSquarePlus class="size-4 shrink-0 text-primary" />
    {#if !rail}<span>{m.shell_sidebar_new_chat()}</span><span class="ml-auto text-[11px] text-sidebar-foreground/50">{$companionName}</span>{/if}
  </button>

  <!-- 对话列表在最上面，占满剩余高度（ADR 028 W11） -->
  {#if !rail}
    <div class="mt-3 flex min-h-0 flex-1 flex-col" data-testid="sidebar-conversations">
      {#if legacyVoiceOnly}
        <p class="m-0 px-2.5 py-2 text-[11px] leading-relaxed text-sidebar-foreground/50" data-testid="sidebar-conversations-empty">{m.shell_sidebar_conversations_empty({ name: $companionName })}</p>
      {:else}
        <ConversationList
          conversations={chat.state?.conversations ?? []}
          currentId={chat.currentId}
          runningId={chat.runningId}
          activePage={page === "assistant" && chatActive}
          onNew={() => void guarded(() => chat.newConversation(), (error) => m.shell_sidebar_new_chat_failed({ error }))}
          onSelect={(id) => {
            if (id === chat.currentId) { onOpenChat(); return; }
            void guarded(() => chat.select(id), (error) => m.shell_sidebar_switch_chat_failed({ error }));
          }}
          onDelete={(id) => chat.remove(id).catch((error) => { toast(false, m.shell_sidebar_delete_chat_failed({ error: errText(error) })); })}
        />
      {/if}
    </div>
  {:else}
    <div class="flex-1"></div>
  {/if}

  <!-- 分组固定在底部，始终看得见 -->
  <nav class="mt-3 flex shrink-0 flex-col gap-px border-t border-hairline pt-2" aria-label={m.shell_sidebar_nav_label()}>
    {#each groups as group, index (group.label)}
      {#if rail}
        {#if index > 0}<div class="mx-2 my-1.5 border-t border-hairline"></div>{/if}
      {:else}
        <div class="flex items-center justify-between gap-2 pr-1 pl-2.5 pb-1 {index > 0 ? 'pt-3' : 'pt-1'}">
          <p class="m-0 text-[11px] font-medium text-sidebar-foreground/50" data-testid={`sidebar-group-${index}`}>{group.label}</p>
          {#if index === 0}{@render allPagesButton()}{/if}
        </div>
      {/if}
      {#each group.items as item (item.id)}{@render editableItem(item)}{/each}
    {/each}
    {#if rail || !groups.length}{@render allPagesButton()}{/if}
  </nav>

  <div class="mt-2 flex shrink-0 flex-col gap-px border-t border-hairline pt-2">
    <!-- 开着的插件放在左侧栏底部的一小块（例如放歌的小播放条，ADR 034、038） -->
    {#each enabledSlots($pluginsStore, $settingsStore, "sidebarFooter") as slot (slot.plugin.manifest.id)}
      <slot.component plugin={slot.plugin} {rail} onOpen={() => onNavigate(pluginPageId(slot.plugin.manifest.id))} />
    {/each}
    {#each footItems as item (item.id)}{@render navItem(item)}{/each}
    {#if !rail}
      <div class="mt-2 flex items-center gap-2 px-2.5 text-[11px] text-sidebar-foreground/45">
        <img src={iconSrc} alt="" class="size-4 rounded object-cover opacity-80" />
        <span class="truncate">{m.shell_sidebar_feedback_group()}</span>
      </div>
    {/if}
  </div>
</aside>
