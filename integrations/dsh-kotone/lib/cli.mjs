// 调用 kotone-cli：发现实例、鉴权与进程身份校验都由 CLI 完成，插件不自己读描述文件或令牌。
import { spawn } from 'node:child_process';
import readline from 'node:readline';

/** kotone-cli 可执行文件：KOTONE_CLI，缺省在 PATH 里找。 */
export function cliPath(env = process.env) {
  return env.KOTONE_CLI || 'kotone-cli';
}

function failure(code, message) {
  return { ok: false, error: { code, message } };
}

/**
 * 调用一个 Kotone 工具，返回结果信封 `{ ok, result?, error? }`（不抛异常）。
 * 参数经标准输入传给 `kotone-cli tools call <tool> --args-file -`。
 */
export function callTool(tool, args = {}, { timeoutMs = 5000, cli = cliPath() } = {}) {
  return new Promise((resolve) => {
    let child;
    try {
      child = spawn(cli, ['tools', 'call', tool, '--args-file', '-', '--timeout-ms', String(timeoutMs)], {
        stdio: ['pipe', 'pipe', 'pipe'],
        windowsHide: true,
      });
    } catch (error) {
      resolve(failure('CLI_UNAVAILABLE', String(error?.message || error)));
      return;
    }
    let out = '';
    let err = '';
    child.stdout.setEncoding('utf8').on('data', (d) => (out += d));
    child.stderr.setEncoding('utf8').on('data', (d) => (err += d));
    child.on('error', (error) => resolve(failure('CLI_UNAVAILABLE', String(error?.message || error))));
    child.on('close', (code) => {
      try {
        const envelope = JSON.parse(out);
        if (envelope && typeof envelope.ok === 'boolean') {
          resolve(envelope);
          return;
        }
      } catch {
        /* 不是信封：落到下面 */
      }
      resolve(failure('CLI_FAILED', `kotone-cli 退出码 ${code}：${(err || out).trim().slice(0, 500)}`));
    });
    child.stdin.end(JSON.stringify(args));
  });
}

/**
 * 持续读取事件（`kotone-cli tools events --follow`）。每个事件调用一次 `onEvent`；
 * 返回 CLI 的退出码（Kotone 未运行 / 退出 / 游标失效时 CLI 退出）。`signal` 中止时结束子进程。
 */
export function followEvents({ types, from = 'oldest' }, onEvent, { signal, cli = cliPath() } = {}) {
  return new Promise((resolve) => {
    if (signal?.aborted) {
      resolve(null);
      return;
    }
    const args = ['tools', 'events', '--follow', '--from', from];
    if (types) args.push('--types', types);
    let child;
    try {
      child = spawn(cli, args, { stdio: ['ignore', 'pipe', 'pipe'], windowsHide: true });
    } catch {
      resolve(-1);
      return;
    }
    const onAbort = () => child.kill();
    signal?.addEventListener('abort', onAbort, { once: true });
    const lines = readline.createInterface({ input: child.stdout });
    lines.on('line', (line) => {
      let event;
      try {
        event = JSON.parse(line);
      } catch {
        return; // 失败信封是多行 JSON，逐行解析不成，忽略
      }
      if (event && typeof event.type === 'string' && typeof event.seq === 'number') onEvent(event);
    });
    child.stderr.resume();
    child.on('error', () => {});
    child.on('close', (code) => {
      signal?.removeEventListener('abort', onAbort);
      resolve(code);
    });
  });
}
