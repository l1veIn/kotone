<script lang="ts">
  // 显示一个插件的 MCP App（ADR 039 D5；MCP Apps 规范 2026-01-26）：iframe 指向回环代理上的沙盒代理页（每个插件一个源，
  // 和 Kotone 的页面、IPC 隔开），用 ext-apps 的 AppBridge（锁 2.1.0）和 App 说话。
  //
  // 握手严格照规范（吸取 ext-apps #671）：沙盒代理页说 ready → 宿主发 sandbox-resource-ready → App 发 ui/initialize →
  // 宿主回宿主上下文 → App 发 initialized；**之后**宿主才推工具的输入和结果、主题和尺寸的变化。
  // - 宿主上下文：主题（亮 / 暗）、语言、显示方式（插件页 = fullscreen，聊天卡片 = inline）、容器尺寸；推了结果时带 toolInfo；
  // - App 调工具：宿主按这个 iframe 是哪个插件的来绑定（后端 `plugin_app` 给的 plugin），只认这个插件给页面的工具，
  //   名字认 `kotone.exe mcp` 对外的那套（$lib/mcpApps）；读资源也只读这个插件的；
  // - 打开链接：外链交给系统浏览器（和 Kotone 别处一样，不在 App 里跳）；`ui://kotone/<本插件>/<页面>?参数` 在 Kotone 里打开；
  // - 请求换显示方式：聊天卡片请求 fullscreen 时在右侧栏打开完整页面，自己还是 inline；pip 先不支持（方案第 4 期映射到覆盖层）；
  // - `ui/message`（替玩家往聊天里发话）：先不给，回 isError（往聊天和她的上下文里放东西是大权力，ADR 039 D3）；
  // - `ui/update-model-context`：接住、回成功，但不存、不进她的上下文（同上，以后再定；宿主能力里也不声明它）；
  // - inline：进入可视区才加载（长对话里不一下子起一堆 iframe），高度跟 App 报的走（有上限）。
  import { onMount } from "svelte";
  import { get } from "svelte/store";
  import type { AppBridge, McpUiHostContext } from "@modelcontextprotocol/ext-apps/app-bridge";
  import { openExternal } from "$lib/ipc";
  import { resolvedTheme } from "$lib/appearance/appearanceRuntime";
  import { currentLocale } from "$lib/locale";
  import { callAppTool, mcpToolName, openApp, parseAppLink, pluginInfo, readAppResource, toCallToolResult, type CallToolResult } from "$lib/mcpApps";
  import { pluginCall } from "$lib/plugins";
  import { FROM_PAGE } from "$lib/pluginUi";
  import { m } from "$lib/paraglide/messages.js";

  let { uri, pluginId = null, displayMode, data = null, params = {}, onOpenPage, onRequestFullscreen }: {
    /** `ui://kotone/<插件>/<页面>`；第三方插件的是它的服务器给的 `ui://` 地址。 */
    uri: string;
    /** 知道是哪个插件的页面时写上（第三方插件的插件页）：两个服务器给了同一个地址时后端不猜。 */
    pluginId?: string | null;
    /** fullscreen：插件页、右侧栏（宽度占满，高度跟 App 报的走、至少 22rem，外面的页面滚）；inline：聊天卡片（高度跟 App 报的走，有上限）。 */
    displayMode: "inline" | "fullscreen";
    /**
     * 工具结果给页面的数据（structuredContent）：有就在握手后推给 App。null 而 `params` 有东西（她 `ui.open` 带参数打开、
     * 别处跳过来）时宿主先替 App 调这一页的工具再推（App 照样只认推来的结果）；都没有 = App 自己去取。
     */
    data?: unknown;
    /** 工具的参数（推结果前先推 tool-input）。 */
    params?: Record<string, unknown>;
    /** App 要打开本插件的另一个页面。 */
    onOpenPage?: (uri: string, params: Record<string, unknown>) => void;
    /** 聊天卡片里的 App 请求全屏：宿主在右侧栏打开完整页面。 */
    onRequestFullscreen?: () => void;
  } = $props();

  /** 高度范围（px）：卡片 120–520；完整页面至少 352（22rem），不设上限。 */
  const INLINE_MIN = 120;
  const INLINE_MAX = 520;
  const FULL_MIN = 352;
  /** App 多久没完成握手算打不开。 */
  const HANDSHAKE_MS = 20_000;

  let container: HTMLDivElement;
  let phase = $state<"waiting" | "loading" | "ready" | "failed">("waiting");
  let error = $state("");
  /** App 报的高度（0 = 还没报）；显示时按显示方式夹在范围里。 */
  let height = $state(0);
  const shownHeight = $derived(displayMode === "inline" ? Math.max(INLINE_MIN, Math.min(INLINE_MAX, height)) : Math.max(FULL_MIN, height));
  let plugin = $state("");

  const plain = (value: unknown): unknown => JSON.parse(JSON.stringify(value ?? null));

  const dimensions = () => {
    const width = Math.round(container?.clientWidth ?? 0) || undefined;
    return displayMode === "inline" ? { width, maxHeight: INLINE_MAX } : { width };
  };

  async function start(): Promise<() => void> {
    phase = "loading";
    let disposed = false;
    let bridge: AppBridge | null = null;
    let frame: HTMLIFrameElement | null = null;
    let initialized = false;
    const cleanups: (() => void)[] = [];
    const dispose = () => {
      disposed = true;
      for (const cleanup of cleanups) cleanup();
      if (bridge && initialized) void bridge.teardownResource({}).catch(() => {});
      void bridge?.close().catch(() => {});
      frame?.remove();
    };
    try {
      const app = await openApp(uri, pluginId);
      const { AppBridge, PostMessageTransport } = await import("@modelcontextprotocol/ext-apps/app-bridge");
      if (disposed) return dispose;
      plugin = app.plugin;
      // 推了结果就告诉 App 是哪个工具开的它（App 等结果，不自己再取一遍）
      const args = Object.fromEntries(Object.entries(plain(params) as Record<string, unknown>).filter(([key]) => key !== FROM_PAGE));
      const withArgs = data == null && Object.keys(args).length > 0;
      const info = data != null || withArgs ? await pluginInfo(app.plugin) : null;
      // 这一页的工具（她调的、或只给页面用的，例如单局详情的 lol.game）
      const pageTool = info?.tools.find((t) => t.page === uri && (data != null || t.visibility.includes("app")))?.name;
      let pushed: CallToolResult | null = data != null ? { content: [], structuredContent: plain(data) as Record<string, unknown> } : null;
      if (withArgs && pageTool) {
        try {
          pushed = toCallToolResult(await pluginCall(app.plugin, pageTool, args));
        } catch (e) {
          console.debug("[mcp app] page tool", pageTool, e);
        }
      }
      if (disposed) return dispose;
      const tool = pushed ? pageTool : undefined;

      frame = document.createElement("iframe");
      frame.setAttribute("sandbox", "allow-scripts allow-same-origin allow-forms");
      frame.setAttribute("referrerpolicy", "no-referrer");
      frame.setAttribute("data-testid", "mcp-app-iframe");
      frame.title = uri;
      frame.style.cssText = "display:block;width:100%;height:100%;border:0;background:transparent;color-scheme:normal";
      container.appendChild(frame);

      let context: McpUiHostContext = {
        theme: get(resolvedTheme),
        locale: currentLocale(),
        displayMode,
        availableDisplayModes: ["inline", "fullscreen"],
        platform: "desktop",
        userAgent: `Kotone/${__APP_VERSION__}`,
        containerDimensions: dimensions(),
        ...(tool ? { toolInfo: { tool: { name: mcpToolName(tool), inputSchema: { type: "object" } } } } : {}),
      };
      const b = new AppBridge(
        null,
        { name: "Kotone", version: __APP_VERSION__ },
        { openLinks: {}, serverTools: {}, serverResources: {}, logging: {} },
        { hostContext: context },
      );
      bridge = b;
      // 处理函数都在 connect 之前挂好：App 握手完马上就可能来请求
      b.oncalltool = async (request) => callAppTool(app.plugin, request.name, (request.arguments ?? {}) as Record<string, unknown>) as never;
      b.onreadresource = async (request) => ({ contents: [await readAppResource(app.plugin, request.uri)] }) as never;
      b.onopenlink = async ({ url }) => {
        const link = parseAppLink(url, app.plugin);
        if (!link) return { isError: true };
        if (link.kind === "external") await openExternal(link.url);
        else onOpenPage?.(link.uri, link.params);
        return {};
      };
      b.onmessage = async () => ({ isError: true });
      b.onupdatemodelcontext = async () => ({});
      b.onrequestdisplaymode = async ({ mode }) => {
        if (mode === "fullscreen" && displayMode === "inline") onRequestFullscreen?.();
        return { mode: displayMode };
      };
      b.addEventListener("sizechange", ({ height: h }) => {
        if (typeof h === "number") height = Math.ceil(h);
      });
      b.addEventListener("loggingmessage", (log) => console.debug("[mcp app]", app.plugin, log.level, log.data));
      b.addEventListener("sandboxready", () => {
        // 沙盒代理页从代理自己的 /app/ 地址加载 App（和 resources/read 同一份，CSP 在响应头里），html 不用带
        void b.sendSandboxResourceReady({ html: "", csp: app.ui.csp });
      });
      b.addEventListener("initialized", () => {
        if (disposed) return;
        initialized = true;
        phase = "ready";
        if (pushed) {
          // 对话记录里的数据是 Svelte 的响应式代理，postMessage 克隆不了：先变成普通对象
          void b.sendToolInput({ arguments: args });
          void b.sendToolResult(pushed as never);
        }
      });

      // 握手以后的变化：主题、容器尺寸
      const update = (patch: McpUiHostContext) => {
        context = { ...context, ...patch };
        if (initialized) b.setHostContext(context);
      };
      cleanups.push(resolvedTheme.subscribe((theme) => update({ theme })));
      const observer = new ResizeObserver(() => update({ containerDimensions: dimensions() }));
      observer.observe(container);
      cleanups.push(() => observer.disconnect());
      const timer = setTimeout(() => {
        if (!initialized && !disposed) {
          phase = "failed";
          error = "timeout";
          frame?.remove();
        }
      }, HANDSHAKE_MS);
      cleanups.push(() => clearTimeout(timer));

      await b.connect(new PostMessageTransport(frame.contentWindow!, frame.contentWindow!));
      frame.src = app.sandboxUrl;
    } catch (e) {
      if (!disposed) {
        phase = "failed";
        error = e instanceof Error ? e.message : String(e);
      }
    }
    return dispose;
  }

  onMount(() => {
    let dispose: (() => void) | null = null;
    let gone = false;
    const begin = () => {
      void start().then((d) => {
        if (gone) d();
        else dispose = d;
      });
    };
    if (displayMode === "fullscreen") {
      begin();
    } else {
      // 聊天卡片：进入可视区才加载
      const seen = new IntersectionObserver((entries) => {
        if (entries.some((e) => e.isIntersecting)) {
          seen.disconnect();
          begin();
        }
      }, { rootMargin: "200px" });
      seen.observe(container);
      dispose = () => seen.disconnect();
    }
    return () => {
      gone = true;
      dispose?.();
    };
  });
</script>

<div
  bind:this={container}
  class="relative w-full overflow-hidden"
  style:height={`${shownHeight}px`}
  data-testid="mcp-app-frame"
  data-uri={uri}
  data-plugin={plugin}
  data-display={displayMode}
  data-phase={phase}
>
  {#if phase === "waiting" || phase === "loading"}
    <div class="pointer-events-none absolute inset-0 animate-pulse rounded-lg bg-muted/40"></div>
  {:else if phase === "failed"}
    <p class="m-0 px-1 py-6 text-center text-xs text-muted-foreground" data-testid="mcp-app-failed" title={error}>{m.chat_plugin_page_missing()}</p>
  {/if}
</div>
