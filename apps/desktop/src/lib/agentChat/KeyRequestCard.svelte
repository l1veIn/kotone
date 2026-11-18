<script lang="ts">
  // 琴音请你粘贴某个服务商的 Key（connections.request_key）：Key 直接存进系统凭据库，
  // 不进聊天记录、不发给 AI；琴音只知道“已保存”。
  import { companionName } from "$lib/stores/companion";
  import { KeyRound } from "@lucide/svelte";
  import { Button } from "$lib/components/ui/button";
  import { agentSubmitKey, type KeyRequest } from "../agent";
  import { errText } from "../stores/ui";

  let { request }: { request: KeyRequest } = $props();
  let key = $state("");
  let busy = $state(false);
  let error = $state("");

  async function submit(value: string | null) {
    if (busy) return;
    busy = true;
    error = "";
    try { await agentSubmitKey(request, value); }
    catch (cause) { error = errText(cause); }
    finally { busy = false; }
  }
</script>

<div class="rounded-2xl border border-primary/40 bg-composer px-4 py-3 text-[13px]" data-testid="agent-key-request" data-provider={request.provider}>
  <p class="m-0 flex items-center gap-2 font-medium text-foreground">
    <KeyRound class="size-4 text-primary" />{$companionName}需要 {request.displayName} 的 Key
  </p>
  {#if request.reason}<p class="mt-1 mb-0 text-xs text-muted-foreground">{request.reason}</p>{/if}
  <p class="mt-1 mb-0 text-xs text-muted-foreground">
    粘贴在这里：直接存进系统凭据库，不会出现在聊天里，也不会发给 AI。
    {#if request.apiKeyUrl}<a class="text-primary hover:underline" href={request.apiKeyUrl} target="_blank" rel="noreferrer">去创建 Key →</a>{/if}
  </p>
  <form class="mt-2.5 flex flex-wrap items-center gap-2" onsubmit={(e) => { e.preventDefault(); if (key.trim()) void submit(key); }}>
    <input type="password" autocomplete="off" spellcheck="false" bind:value={key} placeholder="sk-…" aria-label={`${request.displayName} 的 Key`}
      class="h-8 min-w-0 flex-1 rounded-lg border border-input bg-transparent px-2.5 font-mono text-xs outline-none focus-visible:border-ring"
      data-testid="agent-key-input" />
    <Button size="sm" type="submit" disabled={busy || !key.trim()} data-testid="agent-key-save">{busy ? "保存中…" : "保存"}</Button>
    <Button size="sm" variant="ghost" type="button" disabled={busy} onclick={() => void submit(null)} data-testid="agent-key-skip">不用了</Button>
  </form>
  {#if error}<p role="alert" class="mt-1.5 mb-0 text-xs text-destructive">{error}</p>{/if}
</div>
