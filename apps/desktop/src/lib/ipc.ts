/**
 * 桌面 IPC 的公共入口：保持类型化命令封装与既有导出兼容。
 * 契约定义在 ipc/contracts，dev:web 的内存实现位于 ipc/browserMock。
 */
import { invoke } from "@tauri-apps/api/core";
import defaultLolBlocklistCsv from "../../../../crates/kotone-postprocess/assets/lol-zh-cn-starter.csv?raw";
import * as browserMock from "./ipc/browserMock";
import type {
  PipelineConfig,
  PostProcessingTestResult,
  Settings,
  RuntimeStatus,
  StartupOptions,
  ModelsDirInfo,
  ModelsDirMigration,
  DeleteOutcome,
  SettingsPatch,
  AudioDevice,
  EngineInfo,
  PostProcessorInfo,
  Connection,
  ConnectionInfo,
  ConnectionPreset,
  GameProfile,
  ModelInfo,
  ModelInstallGuide,
  HistoryRecord,
  HotwordMergeReport,
  ElevationStatus,
  HotkeyStatus,
  InputEnvironmentCheck,
  ResourceUsage,
  DiagnosticExportResult,
  DiagnosticWindow,
} from "./ipc/contracts";
import { modelsDirPathError } from "./ipc/contracts";

export * from "./ipc/contracts";

/** 是否运行在 Tauri WebView 中（否则为纯浏览器调试） */
export const isTauri: boolean =
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

// ================================================================
// 命令封装（与 lib.rs invoke_handler 注册名一致）
// ================================================================

/** 冒烟测试：验证 IPC 通路 */
export async function ping(): Promise<string> {
  if (!isTauri) return browserMock.ping();
  return invoke<string>("ping");
}

/** 当前进程的新手向导启动策略（CLI: --onboarding=auto|always|never）。 */
export async function getStartupOptions(): Promise<StartupOptions> {
  if (!isTauri) return browserMock.getStartupOptions();
  return invoke<StartupOptions>("get_startup_options");
}

export async function getSettings(): Promise<Settings> {
  if (!isTauri) return browserMock.getSettings();
  return invoke<Settings>("get_settings");
}

export async function listPostProcessors(): Promise<PostProcessorInfo[]> {
  if (!isTauri) return browserMock.listPostProcessors();
  return invoke<PostProcessorInfo[]>("list_post_processors");
}

export async function listConnectionPresets(): Promise<ConnectionPreset[]> {
  if (!isTauri) return browserMock.listConnectionPresets();
  return invoke<ConnectionPreset[]>("list_connection_presets");
}

export async function listConnections(): Promise<ConnectionInfo[]> {
  if (!isTauri) return browserMock.listConnections();
  return invoke<ConnectionInfo[]>("list_connections");
}

export async function upsertConnection(
  connection: Connection,
  apiKey?: string,
): Promise<Settings> {
  if (!isTauri) return browserMock.upsertConnection(connection, apiKey);
  return invoke<Settings>("upsert_connection", { connection, apiKey: apiKey ?? null });
}

export async function deleteConnection(id: string): Promise<Settings> {
  if (!isTauri) return browserMock.deleteConnection(id);
  return invoke<Settings>("delete_connection", { id });
}

/**
 * 独立试跑一条 pipeline，不写历史、不触发输入注入，也不受总开关影响。
 * 显式传入 pipeline 可保证结果对应设置页当前展示的步骤快照。
 */
export async function testPostProcessing(
  text: string,
  pipeline?: PipelineConfig,
): Promise<PostProcessingTestResult> {
  if (!isTauri) return browserMock.testPostProcessing(text, pipeline);
  return invoke<PostProcessingTestResult>("test_post_processing", { text, pipeline });
}

/** 启动时损坏配置的恢复提示；只返回一次，浏览器演示模式恒为 null。 */
export async function getSettingsLoadWarning(): Promise<string | null> {
  if (!isTauri) return browserMock.getSettingsLoadWarning();
  return invoke<string | null>("get_settings_load_warning");
}

export async function updateSettings(patch: SettingsPatch): Promise<Settings> {
  if (!isTauri) return browserMock.updateSettings(patch);
  return invoke<Settings>("update_settings", { patch });
}

/** 用户拖动悬浮窗后保存当前位置并切换为 custom。 */
export async function saveOverlayPosition(): Promise<Settings> {
  if (!isTauri) return browserMock.saveOverlayPosition();
  return invoke<Settings>("save_overlay_position");
}

export async function listAudioDevices(): Promise<AudioDevice[]> {
  if (!isTauri) return browserMock.listAudioDevices();
  return invoke<AudioDevice[]>("list_audio_devices");
}

export async function setAudioDevice(id: string): Promise<void> {
  if (!isTauri) return browserMock.setAudioDevice(id);
  return invoke<void>("set_audio_device", { id });
}

export async function listSttEngines(): Promise<EngineInfo[]> {
  if (!isTauri) return browserMock.listSttEngines();
  return invoke<EngineInfo[]>("list_stt_engines");
}

export async function setSttEngine(id: string): Promise<void> {
  if (!isTauri) return browserMock.setSttEngine(id);
  return invoke<void>("set_stt_engine", { id });
}

export async function getEngineOptions(id: string): Promise<unknown> {
  if (!isTauri) return browserMock.getEngineOptions(id);
  return invoke<unknown>("get_engine_options", { id });
}

export async function listProfiles(): Promise<GameProfile[]> {
  if (!isTauri) return browserMock.listProfiles();
  return invoke<GameProfile[]>("list_profiles");
}

export async function saveProfile(profile: GameProfile): Promise<void> {
  if (!isTauri) return browserMock.saveProfile(profile);
  return invoke<void>("save_profile", { profile });
}

/** Preview 状态下确认发送（ADR-006：预览只读，始终发送预览文本） */
export async function confirmSend(): Promise<void> {
  if (!isTauri) return browserMock.confirmSend();
  return invoke<void>("confirm_send");
}

/** 取消当前会话（任意状态回 Idle） */
export async function cancelSession(): Promise<void> {
  if (!isTauri) return browserMock.cancelSession();
  return invoke<void>("cancel_session");
}

/** 手动触发发送（调试/测试用） */
export async function simulateSend(text: string, profileId?: string): Promise<void> {
  if (!isTauri) return browserMock.simulateSend(text, profileId);
  return invoke<void>("simulate_send", { text, profileId: profileId ?? null });
}

// ---------- 提权（UIPI 方案） ----------

/** 导出 profile 热词到 UTF-8 文本（每行一词条），返回条数 */
export async function exportHotwords(profileId: string, path: string): Promise<number> {
  if (!isTauri) return browserMock.exportHotwords(profileId, path);
  return invoke<number>("export_hotwords", { profileId, path });
}

/** 导出内置屏蔽词表到 UTF-8 CSV（两列：屏蔽词,替换词），返回规则条数。 */
export async function exportBuiltinBlocklist(path: string): Promise<number> {
  if (!isTauri) return browserMock.exportBuiltinBlocklist(path);
  return invoke<number>("export_builtin_blocklist", { path });
}

/** 内置屏蔽词表 CSV 原文（用于浏览器端 mock 下载导出）。 */
export function builtinBlocklistCsv(): string {
  return defaultLolBlocklistCsv;
}

/** 从 UTF-8 文本导入热词（合并去重），返回合并报告 */
export async function importHotwords(profileId: string, path: string): Promise<HotwordMergeReport> {
  if (!isTauri) return browserMock.importHotwords(profileId, path);
  return invoke<HotwordMergeReport>("import_hotwords", { profileId, path });
}

/** 导出整包 profile 为 .zip 配置包（profile.json + icon.*） */
export async function exportProfile(profileId: string, path: string): Promise<void> {
  if (!isTauri) return browserMock.exportProfile(profileId, path);
  return invoke<void>("export_profile", { profileId, path });
}

/** 导入 .zip 配置包：后端生成新随机 id 落盘，返回导入后的 profile */
export async function importProfile(path: string): Promise<GameProfile> {
  if (!isTauri) return browserMock.importProfile(path);
  return invoke<GameProfile>("import_profile", { path });
}

/** 删除 profile：内置 = 恢复出厂；导入的 = 永久删除。返回动作类型 */
export async function deleteProfile(profileId: string): Promise<"reset" | "deleted"> {
  if (!isTauri) return browserMock.deleteProfile(profileId);
  const out = await invoke<{ kind: "reset" | "deleted" }>("delete_profile", { profileId });
  return out.kind;
}

/** 读取 profile 图标字节（无图标 → 空）；前端按 icon 扩展名推断 mime 建 Blob URL */
export async function getProfileIcon(profileId: string): Promise<Uint8Array> {
  if (!isTauri) return browserMock.getProfileIcon(profileId);
  const bytes = await invoke<number[]>("get_profile_icon", { profileId });
  return new Uint8Array(bytes);
}

/** 游戏兼容状态：自身/游戏提权状态 + 激活 profile 的独占全屏状态 */
export async function getElevationStatus(): Promise<ElevationStatus> {
  if (!isTauri) return browserMock.getElevationStatus();
  return invoke<ElevationStatus>("get_elevation_status");
}

/** 以管理员身份重启（弹 UAC；成功后当前进程退出，调用方无需后续操作） */
export async function restartAsAdmin(): Promise<void> {
  if (!isTauri) return browserMock.restartAsAdmin();
  return invoke<void>("restart_as_admin");
}

/** 读取「开机自动启动」系统真实状态（HKCU Run 键 Kotone 值是否存在） */
export async function getAutostartEnabled(): Promise<boolean> {
  if (!isTauri) return browserMock.getAutostartEnabled();
  return invoke<boolean>("get_autostart_enabled");
}

/** 设置「开机自动启动」：写入 / 删除 HKCU Run 键（值名与 NSIS 卸载清理一致） */
export async function setAutostartEnabled(enabled: boolean): Promise<void> {
  if (!isTauri) return browserMock.setAutostartEnabled(enabled);
  await invoke<void>("set_autostart_enabled", { enabled });
}

/** 试听内置提示音（设置页「音效」tab）：record/send + 音效 id（缺省按事件默认） */
export async function previewSfx(kind: "record" | "send", soundId?: string): Promise<void> {
  if (!isTauri) return browserMock.previewSfx(kind, soundId);
  await invoke<void>("preview_sfx", { kind, soundId: soundId ?? null });
}

/** 热键注册状态（registered/key/error/backend + 具名辅助热键），设置页热键分区展示占用冲突 */
export async function getHotkeyStatus(): Promise<HotkeyStatus> {
  if (!isTauri) return browserMock.getHotkeyStatus();
  return invoke<HotkeyStatus>("get_hotkey_status");
}

/** 主动输入环境自检；可在引导页到达热键步骤时调用，不要求先启动录入。 */
export async function checkInputEnvironment(): Promise<InputEnvironmentCheck> {
  if (!isTauri) return browserMock.checkInputEnvironment();
  return invoke<InputEnvironmentCheck>("check_input_environment");
}

/** 当前进程的 CPU / 内存占用（调用方按 ~2s 间隔轮询） */
export async function getResourceUsage(): Promise<ResourceUsage> {
  if (!isTauri) return browserMock.getResourceUsage();
  return invoke<ResourceUsage>("get_resource_usage");
}

/** 导出诊断 ZIP；包内不含录音、识别文本或热词。 */
export async function exportDiagnostics(
  path: string,
  window: DiagnosticWindow = "all",
): Promise<DiagnosticExportResult> {
  if (!isTauri) return browserMock.exportDiagnostics(path, window);
  return invoke<DiagnosticExportResult>("export_diagnostics", { path, window });
}

/** 清除本地流程事件与 kotone.log。stopRecording 时同时关闭后续记录。 */
export async function clearDiagnostics(stopRecording: boolean): Promise<void> {
  if (!isTauri) return browserMock.clearDiagnostics(stopRecording);
  await invoke<void>("clear_diagnostics", { stopRecording });
}

/** 将前端已处理/未处理异常写入后端持久日志；后端负责脱敏和截断。 */
export async function logFrontendError(context: string, message: string): Promise<void> {
  if (!isTauri) return browserMock.logFrontendError(context, message);
  return invoke<void>("log_frontend_error", { context, message });
}

/** 静态键位冲突提示（向导热键步骤；常见游戏键位表） */
export async function detectHotkeyConflicts(key: string): Promise<string[]> {
  if (!isTauri) return browserMock.detectHotkeyConflicts(key);
  return invoke<string[]>("detect_hotkey_conflicts", { key });
}

/** 开始热键捕获（设置页「点击录入」）；结果经 kotone://hotkey-capture 事件推送 */
export async function startHotkeyCapture(): Promise<void> {
  if (!isTauri) return browserMock.startHotkeyCapture();
  return invoke<void>("start_hotkey_capture");
}

/** 取消进行中的热键捕获（组件销毁/重新点击的兜底） */
export async function cancelHotkeyCapture(): Promise<void> {
  if (!isTauri) return browserMock.cancelHotkeyCapture();
  return invoke<void>("cancel_hotkey_capture");
}

export async function getRuntimeStatus(): Promise<RuntimeStatus> {
  if (!isTauri) return browserMock.getRuntimeStatus();
  return invoke<RuntimeStatus>("get_runtime_status");
}

/** 启动：warmup 引擎 → 注册热键 → 显示悬浮窗；Running+restartNeeded 时等价重启 */
export async function startRuntime(): Promise<RuntimeStatus> {
  if (!isTauri) return browserMock.startRuntime();
  return invoke<RuntimeStatus>("start_runtime");
}

/** 停止：取消会话 → 注销热键 → 隐藏悬浮窗 → 卸载引擎；Stopped 幂等 */
export async function stopRuntime(): Promise<RuntimeStatus> {
  if (!isTauri) return browserMock.stopRuntime();
  return invoke<RuntimeStatus>("stop_runtime");
}

export async function getModelsDir(): Promise<ModelsDirInfo> {
  if (!isTauri) return browserMock.getModelsDir();
  return invoke<ModelsDirInfo>("get_models_dir");
}

/** 切换模型目录（后端先迁移旧目录内容再写配置；空串 = 恢复默认） */
export async function setModelsDir(dir: string): Promise<ModelsDirMigration> {
  const pathError = modelsDirPathError(dir.trim());
  if (pathError) throw new Error(pathError);
  if (!isTauri) return browserMock.setModelsDir(dir);
  return invoke<ModelsDirMigration>("set_models_dir", { dir });
}

/** 删除已下载模型/运行时；active 模型被删时回退默认 */
export async function deleteModel(id: string): Promise<DeleteOutcome> {
  if (!isTauri) return browserMock.deleteModel(id);
  return invoke<DeleteOutcome>("delete_model", { id });
}

/** 在系统文件管理器中打开模型目录 */
export async function openModelsDir(): Promise<void> {
  if (!isTauri) return browserMock.openModelsDir();
  return invoke<void>("open_models_dir");
}

/** 在系统文件管理器中打开历史记录目录（P2-⑨） */
export async function openHistoryDir(): Promise<void> {
  if (!isTauri) return browserMock.openHistoryDir();
  return invoke<void>("open_history_dir");
}

/** 在系统默认浏览器打开外部链接（dev:web 直接 window.open 新标签页） */
export async function openExternal(url: string): Promise<void> {
  if (!isTauri) return browserMock.openExternal(url);
  return invoke<void>("open_external", { url });
}

/** 全部引擎的模型清单（downloaded 标记） */
export async function listModels(): Promise<ModelInfo[]> {
  if (!isTauri) return browserMock.listModels();
  return invoke<ModelInfo[]>("list_models");
}

/** 下载模型/运行时（进度经 kotone://download 事件推送：{ id, downloaded, total }） */
export async function downloadModel(id: string): Promise<void> {
  if (!isTauri) return browserMock.downloadModel(id);
  return invoke<void>("download_model", { id });
}

/** 取消进行中的模型下载（幂等；已下载部分保留可续传） */
export async function cancelDownload(): Promise<void> {
  if (!isTauri) return browserMock.cancelDownload();
  return invoke<void>("cancel_download");
}

/** 自动下载失败后的手动安装指引（官方页 + 目标目录 + 文件清单） */
export async function getModelInstallGuide(id: string): Promise<ModelInstallGuide> {
  if (!isTauri) return browserMock.getModelInstallGuide(id);
  return invoke<ModelInstallGuide>("get_model_install_guide", { id });
}

/** 在资源管理器中打开该模型应放置的目录 */
export async function openModelDestDir(id: string): Promise<void> {
  if (!isTauri) return browserMock.openModelDestDir(id);
  return invoke<void>("open_model_dest_dir", { id });
}

/** 切换引擎的活动模型（Running 时置 restartNeeded，不自动重启） */
export async function setActiveModel(engineId: string, modelId: string): Promise<void> {
  if (!isTauri) return browserMock.setActiveModel(engineId, modelId);
  return invoke<void>("set_active_model", { engineId, modelId });
}

// ---------- 识别历史（历史记录页） ----------

/** 识别历史列表（新→旧） */
export async function getHistory(): Promise<HistoryRecord[]> {
  if (!isTauri) return browserMock.getHistory();
  return invoke<HistoryRecord[]>("get_history");
}

/** 清空全部识别历史（含音频文件；调用方负责二次确认） */
export async function clearHistory(): Promise<void> {
  if (!isTauri) return browserMock.clearHistory();
  return invoke<void>("clear_history");
}

/** 删除单条历史记录（带录音且不再被其他记录引用时一并删除对应 wav；记录不存在幂等） */
export async function deleteHistoryRecord(sessionId: string, ts: string): Promise<void> {
  if (!isTauri) return browserMock.deleteHistoryRecord(sessionId, ts);
  return invoke<void>("delete_history_record", { sessionId, ts });
}

/** 读取历史记录随附的音频（相对 history/audio/ 的文件名 → wav 字节） */
export async function readHistoryAudio(fileName: string): Promise<Uint8Array> {
  if (!isTauri) return browserMock.readHistoryAudio(fileName);
  const bytes = await invoke<number[]>("read_history_audio", { fileName });
  return new Uint8Array(bytes);
}
