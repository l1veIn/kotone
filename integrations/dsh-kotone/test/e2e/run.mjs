// dsh-kotone 真机端到端：Kotone 桌面端 + DSH web profile + 脚本化模型，全部隔离运行。
//
//   node integrations/dsh-kotone/test/e2e/run.mjs --kotone-exe <kotone.exe> --kotone-cli <kotone-cli.exe> --work <目录>
//
// 流程：后台启动 Kotone（assistant.route=agent）→ 启动 DSH（装好 dsh-kotone 与假模型的隔离 profile）
// → assistant.submit 模拟用户说话 → 断言 user.request 被 dsh-kotone 领取、投进「Kotone」会话、
// 模型经 MCP 调用 kotone_agent_reply、Kotone 交付弹幕 → 退出两端。
//
// 前置（一次性）：DSH_HOME=<work>/dsh-home dsh plugin --profile web add link:<dsh-kotone> link:<fake-llm>
// 隔离：KOTONE_HOME / DSH_HOME / WEBVIEW2_USER_DATA_FOLDER 都在 --work 下；不读真实配置、不调用付费 API。
import { execFileSync, spawn } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const argv = process.argv.slice(2);
const arg = (name) => {
  const i = argv.indexOf(name);
  if (i < 0 || !argv[i + 1]) throw new Error(`缺少 ${name}`);
  return path.resolve(argv[i + 1]);
};
const kotoneExe = arg('--kotone-exe');
const kotoneCli = arg('--kotone-cli');
const work = arg('--work');

const env = {
  ...process.env,
  KOTONE_HOME: path.join(work, 'kotone-home'),
  KOTONE_CLI: kotoneCli,
  WEBVIEW2_USER_DATA_FOLDER: path.join(work, 'webview2'),
  DSH_HOME: path.join(work, 'dsh-home'),
  DSH_PERMISSION_MODE: 'danger-full-access',
  KOTONE_FAKE_LLM_LOG: path.join(work, 'fake-llm.log'),
};
fs.rmSync(env.KOTONE_FAKE_LLM_LOG, { force: true });
fs.mkdirSync(env.KOTONE_HOME, { recursive: true });
fs.mkdirSync(env.WEBVIEW2_USER_DATA_FOLDER, { recursive: true });

const t0 = Date.now();
const log = (...a) => console.log(`[${((Date.now() - t0) / 1000).toFixed(1)}s]`, ...a);
const cli = (...args) => JSON.parse(execFileSync(kotoneCli, args, { env, encoding: 'utf8' }));
function call(tool, args, timeoutMs = 5000) {
  return JSON.parse(
    execFileSync(kotoneCli, ['tools', 'call', tool, '--args-file', '-', '--timeout-ms', String(timeoutMs)], {
      env,
      input: JSON.stringify(args),
      encoding: 'utf8',
    }),
  );
}

let dsh;
let dshOut = '';
let failed = false;
try {
  execFileSync(kotoneCli, ['config', 'set', 'assistant.route', 'agent'], { env, stdio: 'ignore' });
  const launched = cli('app', 'launch', '--exe', kotoneExe, '--enable-tool-service');
  if (!launched.ok) throw new Error(`Kotone 启动失败：${JSON.stringify(launched.error)}`);
  log('Kotone ready', launched.result.status.instanceId, 'route', launched.result.status.app.assistantRoute);

  const patches = [path.join(here, 'e2e.patch.yml')];
  const extra = argv.indexOf('--extra-patch'); // 排障用：例如替换 kotone-mcp 的启动命令
  if (extra >= 0) patches.push(path.resolve(argv[extra + 1]));
  // dsh 是 npm 的 .cmd 垫片，需要经 shell 启动；整条命令自己拼好（路径加引号）
  const command = ['dsh', '--profile', 'web', ...patches.flatMap((p) => ['--patch', `"${p}"`]), '--no-open', '--port', '0'];
  dsh = spawn(command.join(' '), { env, shell: true, stdio: ['ignore', 'pipe', 'pipe'] });
  dsh.stdout.on('data', (d) => (dshOut += d));
  dsh.stderr.on('data', (d) => (dshOut += d));
  for (let i = 0; i < 120 && !/dsh web: http/.test(dshOut); i++) await new Promise((r) => setTimeout(r, 250));
  if (!/dsh web: http/.test(dshOut)) throw new Error(`DSH 没有启动：${dshOut.slice(0, 1000)}`);
  log('DSH ready');

  const cursor = call('event.read', { from: 'latest' }).result.cursor;
  // 等 dsh-kotone 的 follow 子进程连上（它从 oldest 读起，早提交也不会漏）
  await new Promise((r) => setTimeout(r, 2000));
  const submitted = call('assistant.submit', { text: '帮我看看今天的委托做完没有' });
  if (!submitted.ok) throw new Error(`assistant.submit 失败：${JSON.stringify(submitted.error)}`);
  log('submitted:', submitted.result.message);

  const seen = [];
  let next = cursor;
  const deadline = Date.now() + 90_000;
  while (Date.now() < deadline && !seen.some((e) => e.type.startsWith('agent.reply.'))) {
    const got = call('event.read', { cursor: next, types: ['user.request', 'agent.*'], waitMs: 20000 }, 25000);
    if (!got.ok) throw new Error(`event.read 失败：${JSON.stringify(got.error)}`);
    for (const e of got.result.events) {
      seen.push(e);
      log('event', e.type, JSON.stringify(e.data).slice(0, 160));
    }
    next = got.result.cursor;
  }
  const request = seen.find((e) => e.type === 'user.request');
  const claimed = seen.find((e) => e.type === 'agent.request.claimed');
  const reply = seen.find((e) => e.type.startsWith('agent.reply.'));
  const check = (ok, what) => {
    console.log(`${ok ? 'PASS' : 'FAIL'} ${what}`);
    if (!ok) failed = true;
  };
  check(Boolean(request), 'user.request 已发布');
  check(claimed && /^dsh-kotone:/.test(claimed.data.consumer), 'dsh-kotone 领取了请求');
  check(reply?.type === 'agent.reply.delivered', 'Agent 经 MCP 回复且 Kotone 交付成功');
  check(reply?.data.requestId === request?.data.requestId, '回复对应同一请求');
  check(/委托还剩两个/.test(reply?.data.text || ''), '回复内容来自模型的工具调用');
  const calls = fs
    .readFileSync(env.KOTONE_FAKE_LLM_LOG, 'utf8')
    .split(/\r?\n/)
    .filter(Boolean)
    .map((line) => JSON.parse(line))
    .filter((r) => r.call);
  check(
    calls.map((c) => c.call).join(',') === 'mcp__kotone__kotone_app_status,mcp__kotone__kotone_agent_reply',
    `模型经 MCP 依次调用了 ${calls.map((c) => c.call).join(' → ')}`,
  );
  const state = path.join(env.DSH_HOME, 'kotone', 'state.json');
  const sessionId = fs.existsSync(state) ? JSON.parse(fs.readFileSync(state, 'utf8')).sessionId : null;
  check(Boolean(sessionId), `请求投进了复用的 Kotone 会话（${sessionId}）`);
  const status = call('agent.requests', {}).result.requests[0];
  check(status?.status === 'replied', `请求状态 ${status?.status}`);
} catch (error) {
  failed = true;
  console.error('FAIL', error.message);
} finally {
  if (dsh?.pid) {
    try {
      execFileSync('taskkill', ['/PID', String(dsh.pid), '/T', '/F'], { stdio: 'ignore' });
    } catch {
      /* 已退出 */
    }
  }
  fs.writeFileSync(path.join(work, 'dsh.log'), dshOut);
  try {
    const quit = cli('app', 'quit');
    log('Kotone quit', quit.ok);
  } catch {
    /* 未在运行 */
  }
}
process.exit(failed ? 1 : 0);
