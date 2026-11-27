/**
 * dev:web 的第三方插件（MCP 方案第 2 期）：照桌面端 `mcp_servers_cmd.rs` / `remote_plugins.rs` 在内存里模拟
 * `mcp.json` 里的服务器——粘贴的配置认出候选、从 Claude Desktop / VS Code 导入（两份示例配置）、加 / 删 / 信任 /
 * 重新连接 / 填 Key。加进来的服务器“连上”以后有两个工具（`<id>.echo` 标了只读、`<id>.add` 没标）。
 * 测试可以在页面加载前放 `window.__kotoneMcpSeed`（服务器列表）当作手改 `mcp.json` 加的（没信任），
 * 放 `window.__kotoneMcpFileSeed`（`{fileError?, problems?}`）当作 `mcp.json` 读不出来、有没加进来的项。
 * 服务器可以带 `kotoneManifest`（它的 Kotone 清单、不认识的字段、问题，照后端 `describe_manifest`）。
 * 不连任何真的服务器。
 */
import type { PluginInfo } from "../plugins";
import type { KotoneManifestInfo, McpCandidate, McpFileStatus, McpImportSource, McpProblem, RemoteInfo, TrustFacts } from "../mcpServers";
import { getSettings } from "./browserMock";

interface MockServer {
  id: string;
  entry: Record<string, unknown>;
  trusted: boolean;
  /** 填了的 Key。 */
  keys: Record<string, string>;
  status: RemoteInfo["status"];
  error?: string | null;
  /** 有 MCP App 页面（`ui://<id>/app`）。 */
  app?: boolean;
  title?: string;
  icon?: string;
  /** 它的 Kotone 清单（插件标准 0.1；没写 = 没有清单）。`manifest` 里没写的字段用默认值。 */
  kotoneManifest?: Partial<Omit<KotoneManifestInfo, "manifest">> & { manifest?: Partial<KotoneManifestInfo["manifest"]> };
}

/** 照后端 `remote_plugins::describe_manifest`：没写的用默认值（没有清单）。 */
function kotoneManifest(s: MockServer): KotoneManifestInfo {
  const seed = s.kotoneManifest ?? {};
  return {
    source: "none", local: false, newerStandard: false, unknown: [], problems: [], unknownGames: [], hostStandard: "0.1",
    localPath: `C:\\Users\\me\\.kotone\\plugins\\${s.id}\\manifest.json`,
    ...seed,
    manifest: { standard: "0.1", games: [], events: [], watches: [], risk: null, ...seed.manifest },
  };
}

let servers: MockServer[] | null = null;
let file: { fileError: string | null; problems: McpProblem[] } | null = null;

function fileState(): { fileError: string | null; problems: McpProblem[] } {
  if (!file) {
    const seed = (globalThis as { __kotoneMcpFileSeed?: Partial<McpFileStatus> }).__kotoneMcpFileSeed;
    file = { fileError: seed?.fileError ?? null, problems: Array.isArray(seed?.problems) ? [...seed.problems] : [] };
  }
  return file;
}

/** `mcp_servers_status`。 */
export async function status(): Promise<McpFileStatus> {
  const state = fileState();
  return { path: "C:\\Users\\me\\.kotone\\mcp.json", fileError: state.fileError, problems: [...state.problems] };
}

/** `mcp_servers_reload`：预览里没有文件，读不出来的照旧读不出来。 */
export async function reload(): Promise<void> {
  if (fileState().fileError) throw new Error(fileState().fileError ?? "");
}

function all(): MockServer[] {
  if (!servers) {
    const seed = (globalThis as { __kotoneMcpSeed?: MockServer[] }).__kotoneMcpSeed;
    servers = Array.isArray(seed) ? seed.map((s) => ({ ...s, keys: s.keys ?? {}, trusted: s.trusted ?? false, status: s.status ?? "notConnected" })) : [];
  }
  return servers;
}

const SECRET = /KEY|TOKEN|SECRET|PASSWORD|PASSWD|AUTH|CREDENTIAL|COOKIE|BEARER/i;
const placeholders = (text: string) => [...text.matchAll(/\$\{input:([A-Za-z0-9_.-]{1,64})\}/g)].map((x) => x[1]);

function strings(entry: Record<string, unknown>): string[] {
  const values = (key: string) => Object.values((entry[key] as Record<string, unknown>) ?? {}).map(String);
  return [String(entry.command ?? ""), String(entry.url ?? ""), ...((entry.args as unknown[]) ?? []).map(String), ...values("env"), ...values("headers")];
}

function facts(entry: Record<string, unknown>): TrustFacts | { error: string } {
  if (String(entry.type ?? "").toLowerCase() === "sse") return { error: "HTTP+SSE" };
  const keys = [...new Set(strings(entry).flatMap(placeholders))].sort();
  if (typeof entry.url === "string" && !entry.command) {
    return { kind: "http", summary: entry.url, url: entry.url, headerNames: Object.keys((entry.headers as object) ?? {}), keys };
  }
  if (typeof entry.command !== "string" || !entry.command) return { error: "command / url" };
  const args = ((entry.args as unknown[]) ?? []).map(String);
  const runner = /^(npx|uvx|bunx|pipx)$/i.test(entry.command);
  return {
    kind: "stdio",
    summary: [entry.command, ...args].join(" "),
    command: entry.command,
    args,
    cwd: (entry.cwd as string) ?? null,
    package: runner ? (args.find((a) => !a.startsWith("-")) ?? null) : null,
    envNames: Object.keys((entry.env as object) ?? {}),
    keys,
  };
}

/** 明文写着的 Key 换成 `${input:名字}`（和后端 `split_secrets` 一样的规矩）。 */
function splitSecrets(entry: Record<string, unknown>): { entry: Record<string, unknown>; moved: [string, string][] } {
  const out = JSON.parse(JSON.stringify(entry)) as Record<string, unknown>;
  const moved: [string, string][] = [];
  for (const field of ["env", "headers"]) {
    const map = out[field] as Record<string, unknown> | undefined;
    if (!map) continue;
    for (const [name, value] of Object.entries(map)) {
      if (typeof value === "string" && value.trim() && !placeholders(value).length && SECRET.test(name)) {
        moved.push([name, value]);
        map[name] = `\${input:${name}}`;
      }
    }
  }
  return { entry: out, moved };
}

function suggestName(entry: Record<string, unknown>): string {
  const f = facts(entry);
  if ("error" in f) return "mcp-server";
  if (f.kind === "http") {
    const host = (f.url ?? "").split("://")[1]?.split(/[/:?]/)[0] ?? "";
    if (host === "localhost" || /^[\d.]+$/.test(host)) return "local";
    return host.split(".").find((l) => !["www", "mcp", "api", "mcp-api"].includes(l)) ?? host;
  }
  const base = (f.package ?? f.command ?? "").split("/").pop()!.split("@").find(Boolean) ?? "";
  return base.replace(/^(server-|mcp-server-|mcp-)/, "").replace(/(-mcp-server|-mcp|-server)$/, "") || "mcp-server";
}

const BUILT_IN = ["lol", "game-voice", "music", "interpreter", "voice-input", "kotone", "connection", "settings", "game", "browser"];

function suggestId(name: string, taken: (id: string) => boolean): string {
  let id = name.toLowerCase().replace(/[_ .]+/g, "-").replace(/[^a-z0-9-]/g, "").replace(/-+/g, "-").replace(/^-|-$/g, "").slice(0, 28);
  if (!id) id = "mcp-server";
  const usable = (c: string) => !BUILT_IN.includes(c) && !taken(c);
  if (usable(id)) return id;
  for (let n = 2; ; n++) if (usable(`${id}-${n}`)) return `${id}-${n}`;
}

function candidate(name: string, entry: Record<string, unknown>, taken: (id: string) => boolean): McpCandidate {
  const shown = name || suggestName(entry);
  const { moved } = splitSecrets(entry);
  const f = facts(entry);
  const warnings = "envFile" in entry ? ["envFile"] : [];
  const out: McpCandidate = { name: shown, id: suggestId(shown, taken), entry, warnings, movedKeys: moved.map(([k]) => k) };
  if ("error" in f) out.error = f.error;
  else {
    out.facts = { ...f, keys: [...new Set([...f.keys, ...out.movedKeys])].sort() };
    out.alreadyAdded = all().find((s) => JSON.stringify(splitSecrets(s.entry).entry) === JSON.stringify(splitSecrets(entry).entry))?.id ?? null;
  }
  return out;
}

const looksLikeServer = (v: unknown) => !!v && typeof v === "object" && ("command" in (v as object) || "url" in (v as object));

export async function preview(text: string): Promise<{ servers: McpCandidate[] }> {
  let value: unknown;
  const cleaned = text.replace(/^\s*\/\/.*$/gm, "").replace(/,(\s*[}\]])/g, "$1").trim();
  try { value = JSON.parse(cleaned); } catch {
    try { value = JSON.parse(`{${cleaned.replace(/,\s*$/, "")}}`); } catch { throw new Error("JSON"); }
  }
  if (!value || typeof value !== "object" || Array.isArray(value)) throw new Error("JSON");
  const root = value as Record<string, unknown>;
  const map = (root.mcpServers ?? root.servers ?? (root.mcp as Record<string, unknown> | undefined)?.servers) as Record<string, Record<string, unknown>> | undefined;
  const ids: string[] = [];
  const taken = (id: string) => all().some((s) => s.id === id) || ids.includes(id);
  const list: [string, Record<string, unknown>][] = map
    ? Object.entries(map)
    : looksLikeServer(root) ? [["", root]]
    : Object.values(root).every(looksLikeServer) ? (Object.entries(root) as [string, Record<string, unknown>][]) : [];
  if (!list.length) throw new Error("mcpServers / servers");
  return { servers: list.map(([name, entry]) => { const c = candidate(name, entry, taken); ids.push(c.id); return c; }) };
}

export async function importSources(): Promise<{ sources: McpImportSource[] }> {
  const taken = (id: string) => all().some((s) => s.id === id);
  return {
    sources: [
      {
        source: "claude-desktop", name: "Claude Desktop",
        files: [{ path: "C:\\Users\\me\\AppData\\Roaming\\Claude\\claude_desktop_config.json", servers: [
          candidate("time", { command: "uvx", args: ["mcp-server-time", "--local-timezone=Asia/Shanghai"] }, taken),
          candidate("github", { command: "npx", args: ["-y", "@modelcontextprotocol/server-github"], env: { GITHUB_PERSONAL_ACCESS_TOKEN: "ghp_example" } }, taken),
        ] }],
      },
      {
        source: "vscode", name: "VS Code",
        files: [{ path: "C:\\Users\\me\\AppData\\Roaming\\Code\\User\\mcp.json", servers: [
          candidate("weather", { type: "http", url: "https://weather.example.com/mcp", headers: { Authorization: "Bearer ${input:token}" } }, taken),
        ] }],
      },
    ],
  };
}

export async function add(id: string, entry: Record<string, unknown>): Promise<void> {
  if (all().some((s) => s.id === id) || BUILT_IN.includes(id) || !/^[a-z0-9](?:[a-z0-9-]{0,30}[a-z0-9])?$/.test(id)) throw new Error(id);
  const { entry: clean, moved } = splitSecrets(entry);
  const f = facts(clean);
  if ("error" in f) throw new Error(f.error);
  const keys = Object.fromEntries(moved);
  const missing = f.keys.some((k) => !(k in keys));
  all().push({ id, entry: clean, trusted: true, keys, status: missing ? "failed" : "connected", error: missing ? "key" : null });
}

export async function remove(id: string): Promise<void> {
  // 没加进来的项：只从 mcp.json 删掉这一项（和后端一样，同名的内置插件不受影响）
  const state = fileState();
  if (state.problems.some((p) => p.id === id)) {
    state.problems = state.problems.filter((p) => p.id !== id);
    return;
  }
  servers = all().filter((s) => s.id !== id);
}

export async function trust(id: string, trusted: boolean): Promise<void> {
  const s = all().find((x) => x.id === id);
  if (s) s.trusted = trusted;
}

export async function reconnect(id: string): Promise<{ connected: boolean; error?: string }> {
  const s = all().find((x) => x.id === id);
  if (!s) throw new Error(id);
  const f = facts(s.entry);
  const missing = "error" in f ? [] : f.keys.filter((k) => !(k in s.keys));
  s.status = missing.length ? "failed" : "connected";
  s.error = missing.length ? `key: ${missing.join(", ")}` : null;
  return s.error ? { connected: false, error: s.error } : { connected: true };
}

export async function setKey(id: string, name: string, value: string | null): Promise<void> {
  const s = all().find((x) => x.id === id);
  if (!s) throw new Error(id);
  if (value && value.trim()) s.keys[name] = value.trim();
  else delete s.keys[name];
  s.status = "notConnected";
}

/** 插件清单里的第三方插件（`plugins_list` 的后半截）。 */
export async function remotePlugins(): Promise<PluginInfo[]> {
  const settings = await getSettings();
  return all().map((s) => {
    const enabled = settings.plugins?.[s.id]?.enabled ?? true;
    const f = facts(s.entry);
    const known = s.status === "connected" || s.app;
    const tools = known ? [
      { name: `${s.id}.echo`, readOnly: true, visibility: ["model", "app"] as const, page: s.app ? `ui://${s.id}/app` : null, title: "Echo", description: "Echo the text back" },
      { name: `${s.id}.add`, readOnly: false, visibility: ["model", "app"] as const, page: null, title: "Add", description: "Add a note" },
    ] : [];
    const remote: RemoteInfo = {
      status: enabled ? s.status : "notConnected",
      error: s.error ?? null,
      trusted: s.trusted,
      server: known ? { name: s.id, title: s.title ?? null, version: "1.0.0", description: null, icons: s.icon ? [{ src: s.icon }] : [] } : null,
      pages: s.app ? [{ uri: `ui://${s.id}/app`, title: "App", slug: "r0" }] : [],
      toolCount: tools.length,
      skippedTools: [],
      keys: "error" in f ? [] : f.keys.map((name) => ({ name, filled: name in s.keys })),
      fingerprint: "mock",
      toolList: tools.map((t) => ({ ...t, visibility: [...t.visibility] })),
      kotoneManifest: kotoneManifest(s),
      ...("error" in f ? { configError: f.error } : { config: f }),
    };
    return {
      manifest: {
        id: s.id, name: s.id, description: "error" in f ? f.error : f.summary, games: kotoneManifest(s).manifest.games, adapter: null,
        enabledByDefault: true, sources: [], pages: [], agent: { overview: "", rule: "" },
      },
      enabled,
      sources: {},
      tools: tools.map((t) => ({ name: t.name, visibility: [...t.visibility], page: t.page, available: enabled })),
      remote,
    } as PluginInfo;
  });
}
