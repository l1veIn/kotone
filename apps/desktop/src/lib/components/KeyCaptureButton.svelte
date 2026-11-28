<script lang="ts">
  // 点一下就录一个键（和「设置 → 热键」每一行的“录入”同一个录键：键盘组合或鼠标侧键，Esc 取消）。
  // 录到了交给 onCombo（查冲突、保存由调用方做）；用在“再加一个键”和聊天页“准备好了吗”的呼叫键（ADR 054）。
  import { onDestroy } from "svelte";
  import { Button } from "$lib/components/ui/button";
  import { captureHotkey } from "../hotkeyCapture";
  import { toast } from "../stores/ui";
  import { m } from "$lib/paraglide/messages.js";

  let { label, capturingLabel, testid, disabled = false, variant = "outline", onCombo }: {
    label: string;
    capturingLabel: string;
    testid: string;
    disabled?: boolean;
    variant?: "outline" | "default" | "ghost";
    onCombo: (combo: string) => Promise<void> | void;
  } = $props();

  let capturing = $state(false);
  let controller: AbortController | null = null;

  async function start() {
    if (capturing || disabled) return;
    capturing = true;
    const abort = new AbortController();
    controller = abort;
    await captureHotkey((result) => {
      if (abort.signal.aborted) return;
      capturing = false;
      controller = null;
      if (result.kind === "combo") void onCombo(result.combo);
      else if (result.kind === "error") toast(false, result.message);
      else if (result.kind === "timeout") toast(false, m.voice_hotkey_capture_timeout());
    }, { signal: abort.signal });
  }

  onDestroy(() => controller?.abort());
</script>

<Button size="sm" {variant} class="h-7 shrink-0 {capturing ? 'animate-pulse' : ''}" data-testid={testid} data-capturing={capturing ? "yes" : "no"}
  disabled={disabled} onclick={() => void start()}>{capturing ? capturingLabel : label}</Button>
