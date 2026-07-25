<script lang="ts">
  /*
   * 设置窗口视图（index.html#/settings，docs/development.md §5.2 routes/settings）。
   * 分区：交互模式与热键 / 麦克风 / STT 引擎 / 发送行为与游戏 profile。
   * 所有读写走 lib/ipc.ts；浏览器 dev:web 下由 mock 支撑，可纯前端调试。
   */
  import { onDestroy, onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import {
    getSettings,
    updateSettings,
    listAudioDevices,
    setAudioDevice,
    listSttEngines,
    setSttEngine,
    listProfiles,
    getElevationStatus,
    getHotkeyStatus,
    restartAsAdmin,
    isTauri,
    startHotkeyCapture,
    cancelHotkeyCapture,
    type Settings,
    type AudioDevice,
    type EngineInfo,
    type GameProfile,
    type ElevationStatus,
    type HotkeyStatus,
    type HotkeyCaptureEvent,
  } from "../../lib/ipc";

  let settings = $state<Settings | null>(null);
  let devices = $state<AudioDevice[]>([]);
  let engines = $state<EngineInfo[]>([]);
  let profiles = $state<GameProfile[]>([]);
  let elevation = $state<ElevationStatus | null>(null);
  let hotkeyStatus = $state<HotkeyStatus | null>(null);
  let restarting = $state(false);

  /** 热键编辑草稿（保存前不动后端） */
  let hotkeyDraft = $state("");
  let hotkeyMode = $state<"toggle" | "hold">("toggle");
  /** 热键录入捕获中（「点击录入」按钮态） */
  let capturing = $state(false);

  /** 底部反馈条：ok 为绿色提示，否则为错误 */
  let feedback = $state<{ ok: boolean; text: string } | null>(null);
  let loading = $state(true);

  function toast(ok: boolean, text: string) {
    feedback = { ok, text };
    setTimeout(() => {
      if (feedback?.text === text) feedback = null;
    }, 3000);
  }

  function errText(e: unknown): string {
    return typeof e === "string" ? e : e instanceof Error ? e.message : String(e);
  }

  onMount(async () => {
    try {
      [settings, devices, engines, profiles, elevation, hotkeyStatus] = await Promise.all([
        getSettings(),
        listAudioDevices(),
        listSttEngines(),
        listProfiles(),
        getElevationStatus(),
        getHotkeyStatus(),
      ]);
      hotkeyDraft = settings.hotkey.key;
      hotkeyMode = settings.hotkey.mode;
    } catch (e) {
      toast(false, `加载配置失败：${errText(e)}`);
    } finally {
      loading = false;
    }
  });

  /** 权限状态轮询：进页刷新一次 + 每 3s；页面隐藏时暂停，回到前台立即补一次 */
  onMount(() => {
    let timer: ReturnType<typeof setInterval> | null = null;
    const refresh = async () => {
      try {
        elevation = await getElevationStatus();
      } catch {
        /* 轮询失败不打扰用户，下轮再试 */
      }
    };
    const start = () => {
      if (timer === null) timer = setInterval(() => void refresh(), 3000);
    };
    const stop = () => {
      if (timer !== null) {
        clearInterval(timer);
        timer = null;
      }
    };
    const onVisibility = () => {
      if (document.hidden) stop();
      else {
        void refresh();
        start();
      }
    };
    start();
    document.addEventListener("visibilitychange", onVisibility);
    return () => {
      stop();
      document.removeEventListener("visibilitychange", onVisibility);
    };
  });

  /** UIPI：目标游戏提权且自身未提权 → 显示提权横幅 */
  const needsElevation = $derived(
    elevation !== null && !elevation.elevated && elevation.activeGameElevated === true,
  );

  async function onRestartAsAdmin() {
    restarting = true;
    try {
      // 成功后当前进程直接退出；走到 catch 说明用户取消 UAC 或失败
      await restartAsAdmin();
    } catch (e) {
      restarting = false;
      toast(false, errText(e));
    }
  }

  async function onRunAsAdminOnStartChange(e: Event) {
    const runAsAdminOnStart = (e.target as HTMLInputElement).checked;
    try {
      settings = await updateSettings({ runAsAdminOnStart });
      toast(
        true,
        runAsAdminOnStart ? "已开启启动时自动提权（每次启动弹一次 UAC）" : "已关闭启动时自动提权",
      );
    } catch (err) {
      toast(false, `保存失败：${errText(err)}`);
    }
  }

  async function saveHotkey() {
    const key = hotkeyDraft.trim();
    if (!key) {
      toast(false, "热键不能为空");
      return;
    }
    try {
      // 热键变更后端自动重注册（lib.rs update_settings）
      settings = await updateSettings({ hotkey: { key, mode: hotkeyMode } });
      hotkeyDraft = settings.hotkey.key;
      hotkeyMode = settings.hotkey.mode;
      toast(true, `热键已保存并生效：${settings.hotkey.key}（${settings.hotkey.mode}）`);
    } catch (e) {
      toast(false, `保存热键失败：${errText(e)}`);
    } finally {
      // 无论成败都刷新注册状态（失败原因展示在热键分区）
      try {
        hotkeyStatus = await getHotkeyStatus();
      } catch {
        /* 状态刷新失败不覆盖保存反馈 */
      }
    }
  }

  /**
   * 「点击录入」：LL 钩子捕获下一个按键组合（ADR-006 热键捕获）。
   * 结果经 kotone://hotkey-capture 事件推送；捕获期间全局热键匹配暂停，
   * Esc 由钩子层转成取消信号（不走 DOM keydown）。
   */
  async function startCapture() {
    if (capturing) return;
    if (!isTauri) {
      toast(false, "浏览器调试环境不支持热键录入");
      return;
    }
    capturing = true;
    let unlisten: (() => void) | undefined;
    try {
      unlisten = await listen<HotkeyCaptureEvent>("kotone://hotkey-capture", (ev) => {
        unlisten?.();
        unlisten = undefined;
        capturing = false;
        const p = ev.payload;
        if (p.combo) {
          hotkeyDraft = p.combo;
          void saveHotkey();
        } else if (p.cancelled) {
          toast(false, "已取消录入");
        } else {
          toast(false, "录入超时，请重试");
        }
      });
      await startHotkeyCapture();
    } catch (e) {
      unlisten?.();
      capturing = false;
      toast(false, `无法启动热键录入：${errText(e)}`);
    }
  }

  // 组件销毁时兜底取消进行中的捕获（避免钩子停在捕获模式）
  onDestroy(() => {
    if (capturing) void cancelHotkeyCapture();
  });

  async function onHotkeyBackendChange(e: Event) {
    const hotkeyBackend = (e.target as HTMLSelectElement).value as Settings["hotkeyBackend"];
    try {
      // 后端变更后端自动重注册（lib.rs update_settings）
      settings = await updateSettings({ hotkeyBackend });
      toast(true, "热键后端已切换并重注册");
    } catch (err) {
      toast(false, `切换热键后端失败：${errText(err)}`);
    } finally {
      try {
        hotkeyStatus = await getHotkeyStatus();
      } catch {
        /* 状态刷新失败不覆盖切换反馈 */
      }
    }
  }

  /** 当前生效后端的展示名 */
  const backendLabel = $derived(
    hotkeyStatus === null
      ? "检测中…"
      : hotkeyStatus.backend === "llhook"
        ? "LL 钩子（游戏前台可用）"
        : hotkeyStatus.backend === "register"
          ? "RegisterHotKey（系统热键）"
          : "未注册",
  );

  async function onDeviceChange(e: Event) {
    const id = (e.target as HTMLSelectElement).value;
    try {
      await setAudioDevice(id);
      if (settings) settings = { ...settings, audioDeviceId: id };
      toast(true, "麦克风已切换");
    } catch (err) {
      toast(false, `切换麦克风失败：${errText(err)}`);
    }
  }

  async function onEngineSwitch(id: string) {
    try {
      await setSttEngine(id);
      if (settings) settings = { ...settings, sttEngine: id };
      const name = engines.find((en) => en.id === id)?.displayName ?? id;
      toast(true, `已切换到引擎：${name}`);
    } catch (e) {
      toast(false, `切换引擎失败：${errText(e)}`);
    }
  }

  async function onAutoSendChange(e: Event) {
    const autoSend = (e.target as HTMLInputElement).checked;
    try {
      settings = await updateSettings({ autoSend });
      toast(true, autoSend ? "已开启转写后直接发送" : "已改为预览确认后发送");
    } catch (err) {
      toast(false, `保存失败：${errText(err)}`);
    }
  }

  async function onProfileChange(e: Event) {
    const activeProfileId = (e.target as HTMLSelectElement).value || null;
    try {
      settings = await updateSettings({ activeProfileId });
      toast(true, "游戏 profile 已切换");
    } catch (err) {
      toast(false, `保存失败：${errText(err)}`);
    }
  }

  async function onEvalRecordingChange(e: Event) {
    const evalRecording = (e.target as HTMLInputElement).checked;
    try {
      settings = await updateSettings({ evalRecording });
      toast(true, evalRecording ? "已开启评测录档" : "已关闭评测录档");
    } catch (err) {
      toast(false, `保存失败：${errText(err)}`);
    }
  }

  /** 引擎能力标签 */
  function engineTags(en: EngineInfo): { text: string; cls: string }[] {
    const tags: { text: string; cls: string }[] = [];
    if (en.capabilities.streaming) tags.push({ text: "流式", cls: "bg-kotone-cyan/15 text-kotone-cyan" });
    if (en.capabilities.offline) tags.push({ text: "离线", cls: "bg-kotone-violet/15 text-kotone-violet" });
    if (en.capabilities.hotwords) tags.push({ text: "热词", cls: "bg-white/10 text-white/70" });
    if (en.capabilities.gpu) tags.push({ text: "GPU", cls: "bg-white/10 text-white/70" });
    if (!en.isReady) tags.push({ text: "未就绪", cls: "bg-kotone-pink/15 text-kotone-pink" });
    return tags;
  }
</script>

<div class="h-full overflow-auto bg-kotone-deep text-white">
  <div class="mx-auto max-w-xl px-8 py-7">
    <header>
      <h1 class="text-2xl font-bold text-kotone-cyan">Kotone 设置</h1>
      <p class="mt-1 text-xs text-white/45">琴音 · 游戏语音输入 —— 按住热键说话，文字直达游戏聊天框</p>
    </header>

    {#if loading}
      <p class="mt-10 text-sm text-white/50">加载配置中…</p>
    {:else if settings}
      <!-- 交互模式与热键 -->
      <section class="mt-6 rounded-xl bg-white/5 p-4 ring-1 ring-white/10">
        <h2 class="text-sm font-semibold text-kotone-cyan/90">交互模式与热键</h2>
        <div class="mt-3 flex gap-2">
          {#each [{ v: "hold", label: "按住说话", desc: "按下开始，松手结束" }, { v: "toggle", label: "按一下切换", desc: "再按一下结束" }] as m}
            <button
              class="flex-1 rounded-lg px-3 py-2 text-left ring-1 transition {hotkeyMode === m.v
                ? 'bg-kotone-cyan/15 ring-kotone-cyan/60'
                : 'bg-white/5 ring-white/10 hover:bg-white/10'}"
              onclick={() => (hotkeyMode = m.v as "toggle" | "hold")}
            >
              <p class="text-sm font-medium">{m.label}</p>
              <p class="text-[11px] text-white/50">{m.desc}</p>
            </button>
          {/each}
        </div>
        <div class="mt-3 flex items-center gap-2">
          <label class="text-xs text-white/60" for="hotkey-input">热键</label>
          <input
            id="hotkey-input"
            bind:value={hotkeyDraft}
            disabled={capturing}
            class="w-32 rounded-lg bg-white/8 px-2.5 py-1.5 text-sm ring-1 ring-white/15 outline-none placeholder:text-white/30 focus:ring-kotone-cyan/60 disabled:opacity-50"
            placeholder="如 F8 / Alt+V"
            spellcheck="false"
          />
          <button
            class="rounded-lg px-3 py-1.5 text-xs font-semibold ring-1 transition active:scale-95 disabled:opacity-70 {capturing
              ? 'animate-pulse bg-kotone-violet/25 text-kotone-violet ring-kotone-violet/60'
              : 'bg-white/10 text-white/85 ring-white/15 hover:bg-white/20'}"
            disabled={capturing}
            onclick={() => void startCapture()}
          >
            {capturing ? "请按下热键组合…（Esc 取消）" : "点击录入"}
          </button>
          <button
            class="rounded-lg bg-kotone-cyan px-3 py-1.5 text-xs font-semibold text-kotone-deep transition hover:brightness-110 active:scale-95 disabled:opacity-50"
            disabled={capturing}
            onclick={() => void saveHotkey()}
          >
            保存并生效
          </button>
          <span class="text-[11px] text-white/40">当前：{settings.hotkey.key}（{settings.hotkey.mode}）</span>
        </div>
        <div class="mt-3 flex items-center gap-2">
          <label class="text-xs text-white/60" for="hotkey-backend">热键后端</label>
          <select
            id="hotkey-backend"
            class="rounded-lg bg-white/8 px-2.5 py-1.5 text-xs ring-1 ring-white/15 outline-none focus:ring-kotone-cyan/60 [&>option]:bg-kotone-deep"
            value={settings.hotkeyBackend}
            onchange={(e) => void onHotkeyBackendChange(e)}
          >
            <option value="auto">自动（优先 LL 钩子）</option>
            <option value="llhook">LL 钩子</option>
            <option value="register">RegisterHotKey</option>
          </select>
          <span class="text-[11px] text-white/40">当前生效：{backendLabel}</span>
        </div>
        <p class="mt-2 text-[11px] text-white/40">
          部分游戏前台时系统热键（RegisterHotKey）收不到按键；LL 钩子（WH_KEYBOARD_LL）可覆盖该场景，命中时会吞掉热键避免触发游戏内同键绑定。
        </p>
        {#if hotkeyStatus?.error}
          <!-- 热键注册失败：典型原因是旧实例未退出或键位被其他程序占用 -->
          <div class="mt-3 rounded-lg bg-kotone-pink/15 p-2.5 ring-1 ring-kotone-pink/50">
            <p class="text-xs font-medium text-kotone-pink">
              热键注册失败，可能被其他程序或其他 Kotone 实例占用
            </p>
            <p class="mt-0.5 text-[11px] break-all text-white/50">{hotkeyStatus.error}</p>
            <button
              class="mt-2 rounded-lg bg-kotone-pink/80 px-2.5 py-1 text-xs font-semibold text-white transition hover:brightness-110 active:scale-95"
              onclick={() => void saveHotkey()}
            >
              重新注册
            </button>
          </div>
        {/if}
      </section>

      <!-- 麦克风 -->
      <section class="mt-4 rounded-xl bg-white/5 p-4 ring-1 ring-white/10">
        <h2 class="text-sm font-semibold text-kotone-cyan/90">麦克风</h2>
        <select
          class="mt-3 w-full rounded-lg bg-white/8 px-2.5 py-2 text-sm ring-1 ring-white/15 outline-none focus:ring-kotone-cyan/60 [&>option]:bg-kotone-deep"
          value={settings.audioDeviceId}
          onchange={(e) => void onDeviceChange(e)}
        >
          {#each devices as d}
            <option value={d.id}>{d.name}</option>
          {/each}
        </select>
      </section>

      <!-- STT 引擎 -->
      <section class="mt-4 rounded-xl bg-white/5 p-4 ring-1 ring-white/10">
        <h2 class="text-sm font-semibold text-kotone-cyan/90">STT 引擎</h2>
        <div class="mt-3 space-y-2">
          {#each engines as en}
            {@const active = settings.sttEngine === en.id}
            <div
              class="flex items-center gap-3 rounded-lg px-3 py-2.5 ring-1 transition {active
                ? 'bg-kotone-cyan/10 ring-kotone-cyan/50'
                : 'bg-white/5 ring-white/10'}"
            >
              <div class="min-w-0 flex-1">
                <p class="flex items-center gap-2 text-sm font-medium">
                  <span class="truncate">{en.displayName}</span>
                  {#if active}
                    <span class="shrink-0 rounded bg-kotone-cyan/20 px-1.5 py-0.5 text-[10px] text-kotone-cyan">
                      使用中
                    </span>
                  {/if}
                </p>
                <p class="mt-1 flex flex-wrap gap-1">
                  {#each engineTags(en) as tag}
                    <span class="rounded px-1.5 py-0.5 text-[10px] {tag.cls}">{tag.text}</span>
                  {/each}
                  <span class="rounded bg-white/5 px-1.5 py-0.5 text-[10px] text-white/40">
                    {en.capabilities.languages.join(" / ")}
                  </span>
                </p>
              </div>
              {#if !active}
                <button
                  class="shrink-0 rounded-lg px-3 py-1.5 text-xs font-semibold transition active:scale-95 {en.isReady
                    ? 'bg-kotone-cyan text-kotone-deep hover:brightness-110'
                    : 'bg-white/10 text-white/60 hover:bg-white/20'}"
                  onclick={() => void onEngineSwitch(en.id)}
                >
                  {en.isReady ? "切换" : "仍切换"}
                </button>
              {/if}
            </div>
          {/each}
        </div>
        <p class="mt-2 text-[11px] text-white/40">
          未就绪的引擎（模型未下载）切换后按热键会提示错误；联调建议使用 mock-stream。
        </p>
      </section>

      <!-- 权限（UIPI 提权方案，docs/development.md §10 R-1） -->
      <section class="mt-4 rounded-xl bg-white/5 p-4 ring-1 ring-white/10">
        <h2 class="text-sm font-semibold text-kotone-cyan/90">权限</h2>
        <div class="mt-3 flex items-center justify-between">
          <span class="text-sm">当前运行权限</span>
          <span
            class="rounded px-2 py-0.5 text-xs font-semibold {elevation?.elevated
              ? 'bg-kotone-cyan/20 text-kotone-cyan'
              : 'bg-white/10 text-white/60'}"
          >
            {elevation === null ? "检测中…" : elevation.elevated ? "管理员" : "普通用户"}
          </span>
        </div>

        {#if needsElevation}
          <!-- 醒目横幅：游戏提权而自身未提权，UIPI 会丢弃合成输入 -->
          <div class="mt-3 rounded-lg bg-kotone-pink/15 p-3 ring-1 ring-kotone-pink/60">
            <p class="text-sm font-semibold text-kotone-pink">
              检测到游戏以管理员运行，Kotone 需要同等权限才能发送
            </p>
            <p class="mt-1 text-[11px] leading-relaxed text-white/60">
              Windows UIPI 会拦截低权限进程发往高权限游戏的模拟输入。重启后会弹出一次 UAC 确认。
            </p>
          </div>
        {/if}

        {#if elevation && !elevation.elevated}
          <!-- 常驻入口：未提权时始终可手动管理员重启，不依赖上方横幅的检测条件 -->
          <button
            class="mt-3 w-full rounded-lg px-3 py-2 text-sm font-semibold transition active:scale-95 disabled:opacity-50 {needsElevation
              ? 'bg-kotone-pink text-white hover:brightness-110'
              : 'bg-white/10 text-white/80 hover:bg-white/20'}"
            disabled={restarting}
            onclick={() => void onRestartAsAdmin()}
          >
            {restarting ? "正在重启…" : "以管理员身份重启"}
          </button>
          {#if !needsElevation}
            <p class="mt-2 text-[11px] text-white/40">
              当前为普通权限。若目标游戏以管理员运行，发送会被系统拦截，此处会提示提权。
            </p>
          {/if}
        {/if}

        <label class="mt-4 flex cursor-pointer items-center justify-between">
          <span>
            <span class="block text-sm">启动时自动以管理员运行</span>
            <span class="block text-[11px] text-white/45">
              每次启动若未提权则自动重启并弹一次 UAC；取消 UAC 则本次按普通权限运行
            </span>
          </span>
          <input
            type="checkbox"
            class="peer sr-only"
            checked={settings.runAsAdminOnStart}
            onchange={(e) => void onRunAsAdminOnStartChange(e)}
          />
          <span
            class="relative h-5 w-9 shrink-0 rounded-full bg-white/15 transition peer-checked:bg-kotone-cyan/70 after:absolute after:top-0.5 after:left-0.5 after:h-4 after:w-4 after:rounded-full after:bg-white after:transition peer-checked:after:translate-x-4"
          ></span>
        </label>
        {#if settings.runAsAdminOnStart && elevation && !elevation.elevated}
          <p class="mt-2 text-[11px] text-white/45">
            已开启，将在下次启动时生效 ·
            <button
              class="text-kotone-cyan underline underline-offset-2 transition hover:brightness-125 disabled:opacity-50"
              disabled={restarting}
              onclick={() => void onRestartAsAdmin()}
            >
              立即以管理员身份重启
            </button>
          </p>
        {/if}
      </section>

      <!-- 发送行为与游戏 profile -->
      <section class="mt-4 rounded-xl bg-white/5 p-4 ring-1 ring-white/10">
        <h2 class="text-sm font-semibold text-kotone-cyan/90">发送行为</h2>
        <label class="mt-3 flex cursor-pointer items-center justify-between">
          <span>
            <span class="block text-sm">转写完成后直接发送</span>
            <span class="block text-[11px] text-white/45">关闭时先弹出预览，确认/编辑后再发送</span>
          </span>
          <input
            type="checkbox"
            class="peer sr-only"
            checked={settings.autoSend}
            onchange={(e) => void onAutoSendChange(e)}
          />
          <span
            class="relative h-5 w-9 shrink-0 rounded-full bg-white/15 transition peer-checked:bg-kotone-cyan/70 after:absolute after:top-0.5 after:left-0.5 after:h-4 after:w-4 after:rounded-full after:bg-white after:transition peer-checked:after:translate-x-4"
          ></span>
        </label>

        <div class="mt-4">
          <label class="text-xs text-white/60" for="profile-select">游戏 profile</label>
          <select
            id="profile-select"
            class="mt-1.5 w-full rounded-lg bg-white/8 px-2.5 py-2 text-sm ring-1 ring-white/15 outline-none focus:ring-kotone-cyan/60 [&>option]:bg-kotone-deep"
            value={settings.activeProfileId ?? ""}
            onchange={(e) => void onProfileChange(e)}
          >
            <option value="">（不指定）</option>
            {#each profiles as p}
              <option value={p.id}>{p.displayName}（{p.id}）</option>
            {/each}
          </select>
        </div>

        <label class="mt-4 flex cursor-pointer items-center justify-between">
          <span>
            <span class="block text-sm">评测录档</span>
            <span class="block text-[11px] text-white/45">保存每次会话的音频与指标，用于引擎对比评测</span>
          </span>
          <input
            type="checkbox"
            class="peer sr-only"
            checked={settings.evalRecording}
            onchange={(e) => void onEvalRecordingChange(e)}
          />
          <span
            class="relative h-5 w-9 shrink-0 rounded-full bg-white/15 transition peer-checked:bg-kotone-cyan/70 after:absolute after:top-0.5 after:left-0.5 after:h-4 after:w-4 after:rounded-full after:bg-white after:transition peer-checked:after:translate-x-4"
          ></span>
        </label>
      </section>
    {/if}

    <!-- 底部反馈 -->
    <div class="h-10 pt-3">
      {#if feedback}
        <p class="text-xs {feedback.ok ? 'text-kotone-cyan' : 'text-kotone-pink'}">{feedback.text}</p>
      {/if}
    </div>
  </div>
</div>
