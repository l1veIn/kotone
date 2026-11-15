// 复用一个「Kotone」会话：所有用户请求都投进同一会话，Agent 能看到上下文（上一句问了什么）。
// 会话 ID 记在 <DSH_HOME>/kotone/state.json；会话被用户删除时自动新建。
import crypto from 'node:crypto';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

const TITLE = 'Kotone';

export function dshHome(env = process.env) {
  return env.DSH_HOME || path.join(os.homedir(), '.dsh');
}

export function stateFile(env = process.env) {
  return path.join(dshHome(env), 'kotone', 'state.json');
}

function readState(file) {
  try {
    return JSON.parse(fs.readFileSync(file, 'utf8'));
  } catch {
    return {};
  }
}

function writeState(file, state) {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  const tmp = `${file}.${process.pid}.tmp`;
  fs.writeFileSync(tmp, JSON.stringify(state, null, 2));
  fs.renameSync(tmp, file);
}

/**
 * @param controller - Host 的 sessionController（create / rename / prompt / inspect / resolveAgent）
 * @param registry - 可选 workspaceRegistry（让会话挂在可见的工作区下）
 */
export function createSessions({ controller, registry, cwd, file = stateFile() }) {
  async function alive(sessionId) {
    if (!sessionId || typeof controller.inspect !== 'function') return false;
    try {
      const info = await controller.inspect(sessionId);
      return Boolean(info && (info.meta || info.events));
    } catch {
      return false;
    }
  }

  async function create() {
    let workspaceId;
    if (registry && cwd && typeof registry.resolveByPath === 'function') {
      try {
        let workspace = await registry.resolveByPath(cwd);
        if (!workspace && typeof registry.create === 'function') workspace = await registry.create(cwd, TITLE);
        workspaceId = workspace?.id;
      } catch {
        /* 工作区注册失败不阻塞会话创建 */
      }
    }
    if (cwd) fs.mkdirSync(cwd, { recursive: true });
    const created = await controller.create(workspaceId ? { workspaceId } : cwd ? { cwd } : {});
    const sessionId = created?.sessionId || created?.id;
    if (!sessionId) throw new Error('创建会话没有返回 sessionId');
    try {
      await controller.rename?.({ sessionId, title: TITLE });
    } catch {
      /* 改不了标题不影响投递 */
    }
    return sessionId;
  }

  let pending = null;
  async function ensure() {
    const state = readState(file);
    if (await alive(state.sessionId)) return state.sessionId;
    const sessionId = await create();
    writeState(file, { ...state, sessionId });
    return sessionId;
  }

  return {
    /** 投递一条用户请求，返回会话 ID。 */
    async deliver(text) {
      // 并发投递共用一次 ensure，避免同时建出两个会话
      pending = pending || ensure().finally(() => (pending = null));
      const sessionId = await pending;
      try {
        await controller.resolveAgent?.(sessionId);
      } catch {
        /* 冷会话 resume 失败时让 prompt 自己报错 */
      }
      // Host 的 prompt 签名是 (request, signal)；queue：Agent 忙时排在当前回合之后
      await controller.prompt(
        { requestId: crypto.randomUUID(), sessionId, mode: 'queue', content: [{ type: 'text', text }] },
        new AbortController().signal,
      );
      return sessionId;
    },
  };
}
