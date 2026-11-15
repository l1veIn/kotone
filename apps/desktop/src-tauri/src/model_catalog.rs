//! 统一模型清单（docs/product-evolution-game-tools.md §11）中 OCR / TTS 类别的桌面命令。
//!
//! ASR 继续使用原有的 `list_models` / `download_model` / `set_active_model` / `delete_model`，
//! 行为不变；这里只管理 `category = ocr | tts` 的条目。设置页与工具服务（`models.*` 带
//! `category` 参数）调用同一批函数。
//!
//! - 下载与 ASR 共用同一个下载名额与取消标记（`kotone_stt::download`），支持断点续传；
//!   OCR 逐文件校验 SHA-256，Kokoro 沿用 `speech_models` 的整包校验与安全解包。
//! - 手动导入：OCR 包可从用户选择的目录复制，仍逐文件校验后才发布。
//! - 切换：OCR 写 `ocr.activeModelId`；TTS 写 `assistant.speechEngine`。本地模型须已安装。

use std::path::{Path, PathBuf};

use kotone_core::model_catalog::{CatalogModel, ModelCategory};
use kotone_core::settings::Settings;
use kotone_ocr::install::Verifier;
use kotone_stt::{download, model};
use tauri::{AppHandle, Emitter, Manager};

use crate::SharedState;

fn parse_category(category: &str) -> Result<ModelCategory, String> {
    match ModelCategory::parse(category) {
        Some(ModelCategory::Asr) => Err("语音识别模型请使用原有的模型接口".into()),
        Some(other) => Ok(other),
        None => Err(format!("未知模型类别 {category}（可选：asr、ocr、tts）")),
    }
}

fn settings_snapshot(app: &AppHandle) -> Settings {
    app.state::<SharedState>().settings.read().unwrap().clone()
}

fn models_root(settings: &Settings) -> PathBuf {
    model::models_dir_from(settings)
}

/// 校验结果按 (大小, 修改时间) 缓存，列清单时不必每次重算全部哈希。
fn verifier() -> &'static Verifier {
    static VERIFIER: std::sync::OnceLock<Verifier> = std::sync::OnceLock::new();
    VERIFIER.get_or_init(Verifier::new)
}

fn kokoro_dir(models: &Path) -> PathBuf {
    models.join("speech").join(kotone_tts::KOKORO_MODEL_DIR)
}

/// 某个 OCR 连接是否已保存密钥；None = 未绑定或无法读取凭据库。
fn deepseek_has_key(app: &AppHandle, settings: &Settings) -> Option<bool> {
    let id = kotone_ocr::catalog::deepseek_connection_id(settings)?;
    let state = app.state::<SharedState>();
    if !settings.connections.iter().any(|c| c.id == id) {
        return Some(false);
    }
    state.secrets.get(&id).ok().map(|key| key.is_some())
}

/// 列出某个类别的条目（阻塞：本地 OCR 包首次列出时需计算哈希）。
pub fn list_blocking(
    app: &AppHandle,
    category: ModelCategory,
) -> Result<Vec<CatalogModel>, String> {
    let settings = settings_snapshot(app);
    let root = models_root(&settings);
    match category {
        ModelCategory::Asr => Err("语音识别模型请使用 list_models".into()),
        ModelCategory::Ocr => Ok(kotone_ocr::catalog::entries(
            &settings,
            &root,
            verifier(),
            deepseek_has_key(app, &settings),
        )),
        ModelCategory::Tts => {
            let installed = crate::speech_models::get_speech_model_status(app.clone()).installed;
            let system_error = app
                .state::<crate::assistant::AssistantRuntime>()
                .system_speech_error();
            Ok(kotone_tts::catalog::entries(
                &kotone_tts::catalog::CatalogState {
                    active: settings.assistant.speech_engine,
                    kokoro_installed: installed,
                    system_error,
                    openai: crate::tts_online::status(app, kotone_tts::online::OnlineKind::OpenAi),
                    volcano: crate::tts_online::status(
                        app,
                        kotone_tts::online::OnlineKind::Volcano,
                    ),
                },
            ))
        }
    }
}

/// 当前生效的模型 ID（OCR：显式选择或自动选择的结果；TTS：当前语音引擎）。
pub fn active_id(app: &AppHandle, category: ModelCategory) -> Option<String> {
    let settings = settings_snapshot(app);
    match category {
        ModelCategory::Asr => None,
        ModelCategory::Ocr => {
            kotone_ocr::catalog::resolve_active(&settings, &models_root(&settings), verifier())
        }
        ModelCategory::Tts => {
            Some(kotone_tts::catalog::model_of(settings.assistant.speech_engine).to_string())
        }
    }
}

fn ensure_known(category: ModelCategory, id: &str) -> Result<(), String> {
    let known = match category {
        ModelCategory::Asr => false,
        ModelCategory::Ocr => kotone_ocr::catalog::is_known(id),
        ModelCategory::Tts => kotone_tts::catalog::is_known(id),
    };
    if known {
        Ok(())
    } else {
        Err(format!("未知{}模型 {id}", label(category)))
    }
}

fn label(category: ModelCategory) -> &'static str {
    match category {
        ModelCategory::Asr => "语音识别",
        ModelCategory::Ocr => "文字识别",
        ModelCategory::Tts => "语音合成",
    }
}

#[tauri::command]
pub async fn list_catalog_models(
    app: AppHandle,
    category: String,
) -> Result<Vec<CatalogModel>, String> {
    let category = parse_category(&category)?;
    tauri::async_runtime::spawn_blocking(move || list_blocking(&app, category))
        .await
        .map_err(|e| format!("列举模型失败：{e}"))?
}

/// 下载本地模型。进度经 `kotone://download` 事件外发：{ id, downloaded, total }（与 ASR 相同）。
pub async fn download(app: &AppHandle, category: ModelCategory, id: &str) -> Result<(), String> {
    ensure_known(category, id)?;
    match category {
        ModelCategory::Asr => Err("语音识别模型请使用 download_model".into()),
        ModelCategory::Tts => {
            if id != kotone_tts::catalog::KOKORO_MODEL_ID {
                return Err(format!("{id} 无需下载"));
            }
            crate::speech_models::download_speech_model(app.clone())
                .await
                .map(|_| ())
        }
        ModelCategory::Ocr => {
            let package = kotone_ocr::manifest::package(id)
                .ok_or_else(|| format!("{id} 是在线模型，无需下载"))?;
            let settings = settings_snapshot(app);
            let root = models_root(&settings);
            model::validate_models_dir_path(&root)?;
            let dir = kotone_ocr::catalog::package_dir(&root, package);
            let cfg = settings.download.clone();
            let app2 = app.clone();
            let id2 = id.to_string();
            tauri::async_runtime::spawn_blocking(move || {
                let _guard = download::begin_download()?;
                std::fs::create_dir_all(&dir)
                    .map_err(|e| format!("无法创建目录 {}：{e}", dir.display()))?;
                let total = package.total_bytes();
                let mut done = 0u64;
                for file in package.files {
                    let dest = dir.join(file.name);
                    if download::sha256_file(&dest).is_ok_and(|h| h == file.sha256) {
                        done += file.bytes;
                        continue;
                    }
                    let base = done;
                    let emit_app = app2.clone();
                    let emit_id = id2.clone();
                    download::download_resolved(
                        file.url,
                        &dest,
                        Some(file.sha256),
                        &move |downloaded, _| {
                            let _ = emit_app.emit(
                                "kotone://download",
                                serde_json::json!({"id": emit_id, "downloaded": base + downloaded, "total": total}),
                            );
                        },
                        &cfg,
                    )?;
                    done += file.bytes;
                }
                match verifier().check(&dir, package).state {
                    kotone_ocr::install::PackageState::Ready => Ok(()),
                    other => Err(format!("下载后复核未通过：{other:?}")),
                }
            })
            .await
            .map_err(|e| format!("下载任务异常：{e}"))?
        }
    }
}

#[tauri::command]
pub async fn download_catalog_model(
    app: AppHandle,
    category: String,
    id: String,
) -> Result<(), String> {
    let category = parse_category(&category)?;
    download(&app, category, &id).await
}

/// 从本地目录导入 OCR 模型包（逐文件校验大小与 SHA-256，校验失败不发布任何文件）。
pub async fn import(
    app: &AppHandle,
    category: ModelCategory,
    id: &str,
    source: &Path,
) -> Result<(), String> {
    ensure_known(category, id)?;
    if category != ModelCategory::Ocr {
        return Err("目前只支持手动导入文字识别模型".into());
    }
    let package = kotone_ocr::manifest::package(id).ok_or_else(|| format!("{id} 是在线模型"))?;
    let settings = settings_snapshot(app);
    let root = models_root(&settings);
    model::validate_models_dir_path(&root)?;
    let dir = kotone_ocr::catalog::package_dir(&root, package);
    let _guard = download::begin_download()?;
    kotone_ocr::install::install(
        package,
        &dir,
        kotone_ocr::install::InstallSource::FromDir(source),
        &|_, _, _| {},
    )
    .await
    .map(|_| ())
}

#[tauri::command]
pub async fn import_catalog_model(
    app: AppHandle,
    category: String,
    id: String,
    dir: String,
) -> Result<(), String> {
    let category = parse_category(&category)?;
    import(&app, category, &id, Path::new(dir.trim())).await
}

/// 删除本地模型文件；返回 wasActive（删的是当前选中的模型，选择已清除）。
pub fn delete(app: &AppHandle, category: ModelCategory, id: &str) -> Result<bool, String> {
    ensure_known(category, id)?;
    if download::is_active() {
        return Err("有模型正在下载，请等待完成或先取消".into());
    }
    let state = app.state::<SharedState>();
    match category {
        ModelCategory::Asr => Err("语音识别模型请使用 delete_model".into()),
        ModelCategory::Ocr => {
            let package = kotone_ocr::manifest::package(id)
                .ok_or_else(|| "在线模型没有本地文件可删除".to_string())?;
            let was_active = state.settings.read().unwrap().ocr.active_model_id == id;
            let dir =
                kotone_ocr::catalog::package_dir(&models_root(&settings_snapshot(app)), package);
            if dir.exists() {
                std::fs::remove_dir_all(&dir)
                    .map_err(|e| format!("删除目录 {} 失败：{e}", dir.display()))?;
            }
            if was_active {
                state.settings_repository.update(|next| {
                    next.ocr.active_model_id.clear();
                    Ok(())
                })?;
            }
            Ok(was_active)
        }
        ModelCategory::Tts => {
            if id != kotone_tts::catalog::KOKORO_MODEL_ID {
                return Err("系统语音无法删除".into());
            }
            let assistant = app.state::<crate::assistant::AssistantRuntime>();
            let _output = crate::assistant::prepare_model_migration(&assistant)?;
            let dir = kokoro_dir(&models_root(&settings_snapshot(app)));
            if dir.exists() {
                std::fs::remove_dir_all(&dir)
                    .map_err(|e| format!("删除目录 {} 失败：{e}", dir.display()))?;
            }
            let was_active = state.settings.read().unwrap().assistant.speech_engine
                == kotone_core::assistant::SpeechEngine::Kokoro;
            if was_active {
                state.settings_repository.update(|next| {
                    next.assistant.speech_engine = kotone_core::assistant::SpeechEngine::System;
                    Ok(())
                })?;
            }
            Ok(was_active)
        }
    }
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogDeleteOutcome {
    pub was_active: bool,
    pub settings: Settings,
}

#[tauri::command]
pub fn delete_catalog_model(
    app: AppHandle,
    category: String,
    id: String,
) -> Result<CatalogDeleteOutcome, String> {
    let category = parse_category(&category)?;
    let was_active = delete(&app, category, &id)?;
    Ok(CatalogDeleteOutcome {
        was_active,
        settings: settings_snapshot(&app),
    })
}

/// 选用某个模型。本地模型须已安装；在线模型可以先选再配置。
pub fn activate(app: &AppHandle, category: ModelCategory, id: &str) -> Result<Settings, String> {
    ensure_known(category, id)?;
    let entry = list_blocking(app, category)?
        .into_iter()
        .find(|m| m.id == id)
        .ok_or_else(|| format!("未知模型 {id}"))?;
    if entry.backend == kotone_core::model_catalog::CatalogBackend::Local && !entry.downloaded {
        return Err(format!("{} 尚未下载，请先下载再切换", entry.display_name));
    }
    let state = app.state::<SharedState>();
    state.settings_repository.update(|next| {
        match category {
            ModelCategory::Asr => return Err("语音识别模型请使用 set_active_model".into()),
            ModelCategory::Ocr => next.ocr.active_model_id = id.to_string(),
            ModelCategory::Tts => {
                next.assistant.speech_engine =
                    kotone_tts::catalog::engine_of(id).ok_or_else(|| format!("未知模型 {id}"))?;
            }
        }
        Ok(())
    })
}

#[tauri::command]
pub async fn activate_catalog_model(
    app: AppHandle,
    category: String,
    id: String,
) -> Result<Settings, String> {
    let category = parse_category(&category)?;
    tauri::async_runtime::spawn_blocking(move || activate(&app, category, &id))
        .await
        .map_err(|e| format!("切换模型失败：{e}"))?
}

/// 手动安装指引（下载失败时的兜底）：官方页、目标目录与应放置的文件。
pub fn install_guide(
    app: &AppHandle,
    category: ModelCategory,
    id: &str,
) -> Result<model::ModelInstallGuide, String> {
    ensure_known(category, id)?;
    let settings = settings_snapshot(app);
    let root = models_root(&settings);
    match category {
        ModelCategory::Ocr => {
            let package =
                kotone_ocr::manifest::package(id).ok_or_else(|| "在线模型无需安装".to_string())?;
            let dir = kotone_ocr::catalog::package_dir(&root, package);
            Ok(model::ModelInstallGuide {
                id: package.id.into(),
                display_name: package.display_name.into(),
                dest_dir: dir.to_string_lossy().into_owned(),
                dest_subdir: format!("ocr/{}", package.id),
                page_url: "https://www.modelscope.cn/models/RapidAI/RapidOCR/files".into(),
                files: package
                    .files
                    .iter()
                    .map(|f| model::ModelInstallFile {
                        name: f.name.into(),
                        url: f.url.into(),
                        size_bytes: f.bytes,
                    })
                    .collect(),
            })
        }
        _ => Err("该模型不提供手动安装指引".into()),
    }
}

#[tauri::command]
pub fn get_catalog_install_guide(
    app: AppHandle,
    category: String,
    id: String,
) -> Result<model::ModelInstallGuide, String> {
    let category = parse_category(&category)?;
    install_guide(&app, category, &id)
}

#[tauri::command]
pub fn open_catalog_model_dir(app: AppHandle, category: String, id: String) -> Result<(), String> {
    let category = parse_category(&category)?;
    let guide = install_guide(&app, category, &id)?;
    let dir = PathBuf::from(guide.dest_dir);
    std::fs::create_dir_all(&dir).map_err(|e| format!("无法创建目录 {}：{e}", dir.display()))?;
    crate::open_in_file_manager(&dir)
}
