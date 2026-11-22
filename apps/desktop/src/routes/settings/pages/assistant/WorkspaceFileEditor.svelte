<script lang="ts">
  // 编辑琴音工作区里的一份文件（SOUL / USER / MEMORY / 游戏记忆）：读、改、按上限计字、保存。
  // 规则与琴音自己用工具写时一样（上限、注入扫描），在下一段对话生效。
  import { Button } from "$lib/components/ui/button";
  import { Textarea } from "$lib/components/ui/textarea";
  import { workspaceRead, workspaceWrite, type WorkspaceFile } from "../../../../lib/workspace";
  import { errText, toast } from "../../../../lib/stores/ui";
  import { m } from "$lib/paraglide/messages.js";

  let { file, limit, label, placeholder = "", testid, onsaved }: {
    file: WorkspaceFile;
    limit: number;
    label: string;
    placeholder?: string;
    testid: string;
    onsaved?: () => void;
  } = $props();

  let text = $state("");
  let saved = $state("");
  let loading = $state(true);
  let saving = $state(false);
  let error = $state("");
  const count = $derived([...text].length);
  const over = $derived(count > limit);
  const dirty = $derived(text !== saved);

  $effect(() => {
    const target = file;
    let disposed = false;
    loading = true;
    error = "";
    void workspaceRead(target)
      .then((value) => { if (!disposed) { text = value; saved = value; } })
      .catch((e) => { if (!disposed) error = m.companion_workspace_load_failed({ error: errText(e) }); })
      .finally(() => { if (!disposed) loading = false; });
    return () => { disposed = true; };
  });

  async function save() {
    saving = true;
    error = "";
    try {
      await workspaceWrite(file, text);
      saved = text;
      toast(true, m.companion_workspace_saved({ label }));
      onsaved?.();
    } catch (e) {
      error = errText(e);
    } finally {
      saving = false;
    }
  }
</script>

<div class="space-y-2" data-testid={testid}>
  <Textarea bind:value={text} disabled={loading} rows={8} spellcheck={false} aria-label={label} {placeholder}
    class="max-h-80 min-h-32 font-mono text-[12.5px] leading-relaxed" data-testid={`${testid}-text`} />
  <div class="flex items-center gap-3">
    <span class="text-[11px] tabular-nums {over ? 'text-destructive' : 'text-muted-foreground'}" data-testid={`${testid}-count`}>{m.companion_char_count({ count, limit })}</span>
    {#if error}<span role="alert" class="min-w-0 flex-1 truncate text-xs text-destructive" title={error} data-testid={`${testid}-error`}>{error}</span>{:else}<span class="flex-1"></span>{/if}
    {#if dirty}<Button size="sm" variant="ghost" disabled={saving} onclick={() => { text = saved; error = ""; }}>{m.companion_workspace_revert()}</Button>{/if}
    <Button size="sm" disabled={!dirty || over || saving || loading} onclick={() => void save()} data-testid={`${testid}-save`}>{saving ? m.companion_saving() : m.companion_save()}</Button>
  </div>
</div>
