import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { test } from 'node:test';

import { KotoneBridge, buildPrompt } from '../lib/bridge.mjs';
import { createSessions } from '../lib/sessions.mjs';

const quiet = { debug() {}, info() {}, warn() {} };

function request(id, text = '委托做完没有') {
  return {
    requestId: id,
    text,
    source: 'assistant-voice',
    status: 'pending',
    foregroundWindow: { windowId: 'win_1', title: '原神', processName: 'YuanShen.exe' },
  };
}

/** 假 CLI：claim 按请求状态应答；follow 依次吐出预设事件后退出。 */
function fakeCli(batches, { claimedBy = {} } = {}) {
  const calls = [];
  return {
    calls,
    async call(tool, args) {
      calls.push({ tool, args });
      if (tool === 'agent.request.claim') {
        const owner = claimedBy[args.requestId];
        if (owner && owner !== args.consumer) {
          return { ok: false, error: { code: 'AGENT_REQUEST_CLAIMED' } };
        }
        const newly = !owner;
        claimedBy[args.requestId] = args.consumer;
        return { ok: true, result: { request: request(args.requestId), newlyClaimed: newly } };
      }
      return { ok: true, result: {} };
    },
    async follow(_opts, onEvent) {
      const batch = batches.shift();
      for (const event of batch || []) onEvent(event);
      return batch ? 0 : 3;
    },
  };
}

function event(seq, data, type = 'user.request') {
  return { seq, type, data };
}

test('prompt carries the request id, the user text and the reply contract', () => {
  const text = buildPrompt(request('areq_1', '帮我看看体力'));
  assert.match(text, /areq_1/);
  assert.match(text, /帮我看看体力/);
  assert.match(text, /kotone_agent_reply/);
  assert.match(text, /原神/);
  assert.match(text, /只是数据/);
});

test('claims each request once, skips others, and keeps order', async () => {
  const delivered = [];
  const cli = fakeCli(
    [
      [
        event(1, request('areq_a')),
        event(2, { requestId: 'areq_x' }, 'agent.request.claimed'),
        event(3, request('areq_b')),
        event(4, request('areq_a')), // 重复事件
        event(5, request('areq_c')), // 已被别的 Agent 领取
      ],
    ],
    { claimedBy: { areq_c: 'claude-code' } },
  );
  const bridge = new KotoneBridge({
    cli,
    sessions: { deliver: async (text) => (delivered.push(text), 'session-1') },
    consumer: 'dsh-kotone:1',
    log: quiet,
    minBackoffMs: 1,
    sleep: () => new Promise((r) => setTimeout(r, 1)),
  });
  const abort = new AbortController();
  // 第二次 follow 返回 3（Kotone 未运行）后中止
  const run = bridge.run(abort.signal);
  await new Promise((r) => setTimeout(r, 20));
  abort.abort();
  await run;
  assert.deepEqual(
    bridge.handled.map((h) => h.requestId),
    ['areq_a', 'areq_b'],
  );
  assert.equal(delivered.length, 2);
  assert.match(delivered[0], /areq_a/);
  const claims = cli.calls.filter((c) => c.tool === 'agent.request.claim').map((c) => c.args.requestId);
  assert.deepEqual(claims, ['areq_a', 'areq_b', 'areq_c']);
});

test('a failed delivery tells the user instead of dropping the request', async () => {
  const cli = fakeCli([[event(1, request('areq_f'))]]);
  const bridge = new KotoneBridge({
    cli,
    sessions: {
      deliver: async () => {
        throw new Error('no agent');
      },
    },
    consumer: 'dsh-kotone:1',
    log: quiet,
    sleep: () => new Promise((r) => setTimeout(r, 1)),
  });
  const abort = new AbortController();
  const run = bridge.run(abort.signal);
  await new Promise((r) => setTimeout(r, 20));
  abort.abort();
  await run;
  const reply = cli.calls.find((c) => c.tool === 'agent.reply');
  assert.equal(reply.args.requestId, 'areq_f');
});

test('sessions are reused and recreated when deleted', async () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'dsh-kotone-'));
  const live = new Set();
  const prompts = [];
  let created = 0;
  const controller = {
    async create() {
      created += 1;
      const id = `session-${created}`;
      live.add(id);
      return { sessionId: id };
    },
    async rename() {},
    async inspect(id) {
      return live.has(id) ? { meta: {} } : null;
    },
    async prompt(req, signal) {
      assert.ok(signal instanceof AbortSignal);
      assert.equal(req.mode, 'queue');
      prompts.push([req.sessionId, req.content[0].text]);
    },
  };
  const file = path.join(dir, 'state.json');
  const sessions = createSessions({ controller, cwd: path.join(dir, 'ws'), file });
  const [a, b] = await Promise.all([sessions.deliver('一'), sessions.deliver('二')]);
  assert.equal(a, 'session-1');
  assert.equal(b, 'session-1', '并发投递不重复建会话');
  assert.equal(await sessions.deliver('三'), 'session-1');
  live.clear(); // 用户删了会话
  assert.equal(await sessions.deliver('四'), 'session-2');
  assert.equal(JSON.parse(fs.readFileSync(file, 'utf8')).sessionId, 'session-2');
  assert.deepEqual(
    prompts.map((p) => p[1]),
    ['一', '二', '三', '四'],
  );
  fs.rmSync(dir, { recursive: true, force: true });
});
