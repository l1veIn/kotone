// 端到端测试用的脚本化模型（不联网、不需要 API key）：
// 看到「【Kotone 用户请求 areq_…】」就调用 Kotone 的回复工具；拿到工具结果后用一句话收尾。
// 只用于 integrations/dsh-kotone/test/e2e，不要装进日常 profile。
import fs from 'node:fs';
import { createRequire } from 'node:module';
import path from 'node:path';
import { pathToFileURL } from 'node:url';

export const name = 'kotone-fake-llm';
export const inject = ['llm'];

export const PROVIDER = 'kotone-fake';
export const MODEL = 'scripted';
export const REPLY_TEXT = '收到，委托还剩两个。';

/** 从 DSH 安装目录解析 dsh-llm（本包是 link 进来的，按自身路径解析不到）。 */
async function loadLlm() {
  const entry = process.argv[1] || '';
  const anchor = entry.includes(`${path.sep}@deepseek-ai${path.sep}dsh${path.sep}`)
    ? entry.slice(0, entry.indexOf(`${path.sep}@deepseek-ai${path.sep}dsh${path.sep}`) + `${path.sep}@deepseek-ai${path.sep}dsh${path.sep}`.length)
    : path.dirname(entry);
  const require = createRequire(path.join(anchor, 'package.json'));
  return import(pathToFileURL(require.resolve('@deepseek-ai/dsh-llm')).href);
}

function textOf(message) {
  const content = message?.content;
  if (typeof content === 'string') return content;
  return (content || [])
    .filter((b) => b && b.type === 'text')
    .map((b) => b.text)
    .join('\n');
}

function trace(record) {
  if (process.env.KOTONE_FAKE_LLM_LOG) {
    fs.appendFileSync(process.env.KOTONE_FAKE_LLM_LOG, `${JSON.stringify({ at: new Date().toISOString(), ...record })}\n`);
  }
}

export async function apply(ctx) {
  let LlmAdapter;
  try {
    ({ LlmAdapter } = await loadLlm());
  } catch (error) {
    trace({ applyError: String(error?.stack || error), argv: process.argv.slice(0, 3) });
    throw error;
  }
  let callSeq = 0;

  class ScriptedAdapter extends LlmAdapter {
    providerInfo(provider) {
      return { id: provider, name: 'Kotone scripted test model' };
    }
    async listModels() {
      return [{ provider: PROVIDER, id: MODEL, name: 'scripted' }];
    }
    async resolveModel(provider, model) {
      return { provider, id: model, name: model, context: { contextWindow: 128000 }, defaultMaxTokens: 1024 };
    }
    async *stream(options) {
      const messages = options.messages || [];
      const tools = (options.tools || []).map((t) => t.name);
      trace({ roles: messages.map((m) => m.role), kotoneTools: tools.filter((n) => n.includes('kotone')).length });
      const statusTool = tools.find((n) => /kotone_app_status$/.test(n));
      const replyTool = tools.find((n) => /kotone_agent_reply$/.test(n));
      // 最近一条带 Kotone 请求标记的用户消息（之后还可能有宿主注入的上下文消息）
      let markerAt = -1;
      let match = null;
      for (let i = messages.length - 1; i >= 0 && !match; i--) {
        if (messages[i].role !== 'user') continue;
        match = /【Kotone 用户请求 (areq_[0-9a-f]+)】/.exec(textOf(messages[i]));
        if (match) markerAt = i;
      }
      // 本轮已经拿到几个工具结果：0 → 先查 Kotone 状态，1 → 回复用户，之后收尾
      const results = messages.slice(markerAt + 1).filter((m) => m.role === 'tool').length;
      const usage = { type: 'usage', usage: { inputTokens: 1, outputTokens: 1 } };
      const plan =
        match && statusTool && replyTool
          ? [
              [statusTool, {}],
              [replyTool, { requestId: match[1], text: REPLY_TEXT, outputs: ['danmaku'] }],
            ][results]
          : undefined;
      if (plan) {
        const [tool, toolArgs] = plan;
        const id = `call_${++callSeq}`;
        const args = JSON.stringify(toolArgs);
        trace({ call: tool });
        yield { type: 'block-start', index: 0, blockType: 'tool-call' };
        yield { type: 'tool-call-delta', index: 0, id, name: tool, argumentsDelta: args };
        yield { type: 'block-end', index: 0, block: { type: 'tool-call', id, name: tool, arguments: args } };
        yield usage;
        yield { type: 'finish', reason: { kind: 'tool-calls' } };
        return;
      }
      const text = match ? '已经通过 Kotone 回复用户。' : `tools=${tools.length}`;
      yield { type: 'block-start', index: 0, blockType: 'text' };
      yield { type: 'text-delta', index: 0, text };
      yield { type: 'block-end', index: 0, block: { type: 'text', text } };
      yield usage;
      yield { type: 'finish', reason: { kind: 'stop' } };
    }
  }

  ctx.llm.registerAdapter([PROVIDER], new ScriptedAdapter());
  trace({ registered: PROVIDER, providers: ctx.llm.listProviders?.() });
}
