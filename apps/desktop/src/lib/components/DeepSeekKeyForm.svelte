<script lang="ts">
  /*
   * 填 DeepSeek Key（ADR 028 W2 / W5）：引导里的一步和聊天页“没有 AI 连接”时的卡片是同一个组件。
   * 版式照 DeepSeek Harness 欢迎窗的 Key 页：一个密码框，不选服务商；保存并继续 / 稍后配置 / 返回。
   * 比 DSH 多了测试连接和“去开放平台创建”的链接。
   */
  import { openExternal } from "../ipc";
  import { connectDeepSeek, deepseekKeyProblem, DEEPSEEK_KEY_PAGE, type DeepSeekSetupResult } from "../deepseekSetup";
  import { errText, settingsStore } from "../stores/ui";

  let { variant = "dialog", testid = "deepseek-key", onConnected, onLater, onBack, laterLabel = "稍后配置" }: {
    variant?: "dialog" | "card";
    testid?: string;
    onConnected: (result: DeepSeekSetupResult) => void;
    onLater?: () => void;
    onBack?: () => void;
    laterLabel?: string;
  } = $props();

  let key = $state("");
  let busy = $state(false);
  let error = $state("");

  async function submit(event: SubmitEvent) {
    event.preventDefault();
    if (busy) return;
    const problem = deepseekKeyProblem(key);
    if (problem) { error = problem; return; }
    busy = true;
    error = "";
    try {
      const result = await connectDeepSeek(key, $settingsStore);
      settingsStore.set(result.settings);
      key = "";
      onConnected(result);
    } catch (e) {
      error = errText(e);
    } finally {
      busy = false;
    }
  }
</script>

<form class={variant === "card" ? "w-full max-w-sm rounded-2xl border border-hairline bg-card/80 p-4 text-left shadow-sm" : "w-full"}
  data-testid={testid} onsubmit={(event) => void submit(event)}>
  {#if variant === "dialog"}
    <h2 class="m-0 text-lg font-bold">添加一个 DeepSeek Key 开始使用</h2>
  {:else}
    <h2 class="m-0 text-[15px] font-semibold">先添加一个 DeepSeek Key</h2>
  {/if}
  <p class="mt-1.5 mb-0 text-xs leading-relaxed text-muted-foreground">
    有这一个 Key 就能用全部功能：对话、语音识别、文字识别、语音合成、联网搜索。按用量付费；Key 只存进本机的系统凭据库，界面和聊天里都看不到它。
  </p>
  <input
    type="password"
    autocomplete="off"
    spellcheck="false"
    placeholder="sk-…"
    aria-label="DeepSeek Key"
    data-testid={`${testid}-input`}
    bind:value={key}
    disabled={busy}
    oninput={() => (error = "")}
    class="mt-4 h-10 w-full rounded-lg border border-hairline bg-background px-3 font-mono text-sm outline-none focus:border-primary disabled:opacity-60"
  />
  <p class="mt-2 mb-0 min-h-4 text-xs" role={error ? "alert" : "status"} data-testid={`${testid}-status`}>
    {#if error}<span class="text-destructive" data-testid={`${testid}-error`}>{error}</span>
    {:else if busy}<span class="text-primary">正在测试：读取模型列表…</span>{/if}
  </p>
  <button type="button" class="mt-1 text-xs text-primary hover:underline" onclick={() => void openExternal(DEEPSEEK_KEY_PAGE).catch(() => {})}>
    还没有 Key？去 DeepSeek 开放平台创建 →
  </button>
  <div class="mt-5 flex items-center justify-between gap-2">
    {#if onBack}
      <button type="button" class="text-xs text-muted-foreground hover:text-foreground/90" disabled={busy} data-testid={`${testid}-back`} onclick={onBack}>返回</button>
    {:else}<span></span>{/if}
    <div class="flex items-center gap-2">
      {#if onLater}
        <button type="button" class="rounded-lg border border-hairline px-3.5 py-2 text-sm text-foreground/85 transition hover:bg-muted disabled:opacity-50"
          disabled={busy} data-testid={`${testid}-later`} onclick={onLater}>{laterLabel}</button>
      {/if}
      <button type="submit" data-testid={`${testid}-submit`} disabled={busy || !key.trim()}
        class="rounded-lg bg-primary px-5 py-2 text-sm font-semibold text-primary-foreground transition hover:brightness-110 disabled:opacity-50">
        {busy ? "测试中…" : "测试并继续"}
      </button>
    </div>
  </div>
</form>
