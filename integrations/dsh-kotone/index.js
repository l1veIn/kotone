// dsh-kotone · Host 侧：Kotone 的反向通道。
//
// MCP 工具（kotone_*）由 bundle 补丁里的 kotone-mcp 条目（@deepseek-ai/dsh-mcp-client）提供；
// 本插件只负责「用户 → Agent」方向：Kotone 发布 user.request 时领取它、投进「Kotone」会话。
// 依赖 sessionController：没有会话服务的 profile（如 headless）里本插件不激活，
// 也就不会领走用户请求。
import fs from 'node:fs';
import path from 'node:path';

import { callTool, followEvents } from './lib/cli.mjs';
import { KotoneBridge } from './lib/bridge.mjs';
import { createSessions, dshHome } from './lib/sessions.mjs';

export const name = 'kotone';
export const inject = ['sessionController'];

/** 落盘日志 `<DSH_HOME>/kotone/bridge.log`：Host logger 不落盘，转交路径出问题时靠它查证。 */
function fileLog(level, message) {
  try {
    const file = path.join(dshHome(), 'kotone', 'bridge.log');
    fs.mkdirSync(path.dirname(file), { recursive: true });
    fs.appendFileSync(file, `[${new Date().toISOString()}] ${level} ${message}\n`, 'utf8');
  } catch {
    /* 日志写不进去不影响转交 */
  }
}

function makeLogger(ctx) {
  let base = null;
  try {
    const raw = ctx && ctx.logger;
    base = typeof raw === 'function' ? raw('kotone') : raw;
  } catch {
    base = null;
  }
  const pick = (method) => {
    const host =
      base && typeof base[method] === 'function'
        ? base[method].bind(base)
        : typeof console[method] === 'function'
          ? console[method].bind(console)
          : () => {};
    return (message) => {
      fileLog(method, message);
      host(message);
    };
  };
  return { debug: pick('debug'), info: pick('info'), warn: pick('warn') };
}

function service(ctx, key) {
  try {
    const value = typeof ctx.get === 'function' ? ctx.get(key) : undefined;
    if (value) return value;
  } catch {
    /* ignore */
  }
  return ctx[key];
}

export function apply(ctx, config = {}) {
  const log = makeLogger(ctx);
  const controller = service(ctx, 'sessionController');
  if (!controller || typeof controller.prompt !== 'function' || typeof controller.create !== 'function') {
    log.warn('kotone: Host 没有可用的 sessionController，反向通道不启用');
    return;
  }
  const sessions = createSessions({
    controller,
    registry: service(ctx, 'workspaceRegistry'),
    cwd: config.cwd || path.join(dshHome(), 'kotone', 'workspace'),
  });
  const bridge = new KotoneBridge({
    cli: {
      call: (tool, args) => callTool(tool, args),
      follow: (opts, onEvent, signal) => followEvents(opts, onEvent, { signal }),
    },
    sessions,
    consumer: `dsh-kotone:${process.pid}`,
    log,
  });
  ctx.effect(() => {
    const abort = new AbortController();
    // 后台循环必须自己兜住所有异常，否则会把宿主带走
    bridge.run(abort.signal).catch((error) => log.warn(`kotone: 反向通道退出：${error?.message || error}`));
    return () => abort.abort();
  }, 'kotone: 用户请求反向通道');
  log.info('kotone: 已加载（等待 Kotone 的用户请求）');
}
