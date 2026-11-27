//! 模型配置里的 Key（ADR 048）：不进 `config.json`，只存系统凭据库。
//!
//! 以前语音识别里火山的 Access Token、讯飞的 APISecret 明文写在 `modelConfigs`（schema 标的是 `secret`，
//! 实际没人管）。现在：
//! - 名字是 `<类别>:<模型 ID>`（`asr:volcano-asr`、`tts:minimax-tts`），和插件的 `<插件 id>:<名字>`
//!   一个写法（凭据库里再由 `SecretStore` 的实现加上自己的前缀）；一个模型最多一个 Key 字段（[`FIELDS`]）。
//! - 读配置时 [`migrate`]：配置里有明文、凭据库里还没有，就写进凭据库；然后从配置里去掉。写不进去就留着，
//!   不丢玩家的 Key。
//! - 保存设置时 [`take`]：补丁里填了 Key（不是空的）就存进凭据库（覆盖），配置里不留；空的当“不改”。
//! - 用的时候 [`fill`]：会话配置里没有这个字段就从凭据库补上（`stt::add_remote_connection` 调它）。
//!
//! Key 不进日志、错误消息：这里的日志和错误只写名字（`asr:volcano-asr`），不写值。

use std::sync::{Arc, RwLock};

use serde_json::Value;

use crate::connection::SecretStore;
use crate::settings::Settings;

/// 哪个模型的哪个字段是 Key。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SecretField {
    /// `asr` / `tts`（凭据库名字的前缀）。
    pub category: &'static str,
    pub model: &'static str,
    pub field: &'static str,
}

impl SecretField {
    /// 凭据库里的名字：`<类别>:<模型 ID>`。
    pub fn name(&self) -> String {
        format!("{}:{}", self.category, self.model)
    }
}

const fn secret(category: &'static str, model: &'static str, field: &'static str) -> SecretField {
    SecretField {
        category,
        model,
        field,
    }
}

/// 全部 Key 字段。语音识别的两个与 `kotone_stt::model` 的 schema（`kind: "secret"`）一致，有测试对着查。
pub const FIELDS: &[SecretField] = &[
    secret("asr", "volcano-asr", "accessToken"),
    secret("asr", "iflytek-asr", "apiSecret"),
    secret("tts", "volcano-tts", "accessToken"),
    secret("tts", "deepseek-tts", "apiKey"),
    secret("tts", "minimax-tts", "apiKey"),
    secret("tts", "dashscope-tts", "apiKey"),
    secret("tts", "siliconflow-tts", "apiKey"),
    secret("tts", "openai-compat-tts", "apiKey"),
];

/// 这个模型的 Key 字段。
pub fn field_of(model: &str) -> Option<&'static SecretField> {
    FIELDS.iter().find(|f| f.model == model)
}

/// 这个模型的 Key 在凭据库里叫什么（没有 Key 字段的模型是 None）。
pub fn name_of(model: &str) -> Option<String> {
    field_of(model).map(SecretField::name)
}

static STORE: RwLock<Option<Arc<dyn SecretStore>>> = RwLock::new(None);

/// 宿主启动时装上凭据库（桌面端是 Windows 凭据管理器）。没装时 [`get`] 一律是 None。
pub fn install(store: Arc<dyn SecretStore>) {
    *STORE.write().unwrap() = Some(store);
}

/// 装上的凭据库（[`install`]）。
pub fn installed() -> Option<Arc<dyn SecretStore>> {
    STORE.read().unwrap().clone()
}

/// 从装上的凭据库读这个模型的 Key（读不了、没有都是 None）。
pub fn get(model: &str) -> Option<String> {
    let store = installed()?;
    get_from(store.as_ref(), model)
}

/// 从给定的凭据库读这个模型的 Key。
pub fn get_from(store: &dyn SecretStore, model: &str) -> Option<String> {
    let name = name_of(model)?;
    store
        .get(&name)
        .ok()
        .flatten()
        .filter(|key| !key.trim().is_empty())
}

/// [`fill`] 用装上的凭据库（[`install`]）：引擎查“配好了没有”时用。
pub fn fill_installed(options: &mut serde_json::Map<String, Value>, model: &str) {
    let Some(store) = installed() else {
        return;
    };
    let mut value = Value::Object(std::mem::take(options));
    fill(&mut value, [model], store.as_ref());
    if let Value::Object(map) = value {
        *options = map;
    }
}

fn text(value: Option<&Value>) -> Option<&str> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
}

/// 从配置里拿走这个字段：`modelConfigs[<模型>]`，还有更早的写法 `engineOptions[<模型>]`、
/// `engineOptions["online-asr"]`（火山和讯飞共用一个分组，字段名不同）。返回有没有这个字段、第一个非空的明文。
fn take_plain(settings: &mut Settings, field: &SecretField) -> (bool, Option<String>) {
    let mut found = false;
    let mut value: Option<String> = None;
    let mut take_from = |map: Option<&mut serde_json::Map<String, Value>>| {
        if let Some(old) = map.and_then(|m| m.remove(field.field)) {
            found = true;
            if value.is_none() {
                value = text(Some(&old)).map(str::to_string);
            }
        }
    };
    take_from(
        settings
            .model_configs
            .get_mut(field.model)
            .and_then(Value::as_object_mut),
    );
    let groups: &[&str] = if field.category == "asr" {
        &["online-asr"]
    } else {
        &[]
    };
    for key in std::iter::once(field.model).chain(groups.iter().copied()) {
        take_from(
            settings
                .engine_options
                .get_mut(key)
                .and_then(Value::as_object_mut),
        );
    }
    (found, value)
}

/// 读配置时迁走明文（ADR 048）：凭据库里还没有就写进去，然后从配置里去掉；写不进去的那一项原样留着。
/// 返回改了配置没有（改了要存盘）。
pub fn migrate(settings: &mut Settings, store: &dyn SecretStore) -> bool {
    let mut changed = false;
    for field in FIELDS {
        let snapshot = settings.clone();
        let (found, plain) = take_plain(settings, field);
        if !found {
            continue;
        }
        let name = field.name();
        let Some(plain) = plain else {
            // 只有空字段：去掉就行
            changed = true;
            continue;
        };
        let stored = match store.get(&name) {
            Ok(Some(existing)) if !existing.trim().is_empty() => Ok(()),
            Ok(_) => store.set(&name, &plain),
            Err(error) => Err(error),
        };
        match stored {
            Ok(()) => {
                crate::log::log(&format!("配置里 {name} 的 Key 挪进了凭据库"));
                changed = true;
            }
            Err(error) => {
                // 不丢 Key：这一项照旧留在配置里，下次启动再试
                *settings = snapshot;
                crate::log::log(&format!(
                    "{name} 的 Key 没能挪进凭据库，先留在配置里：{error}"
                ));
            }
        }
    }
    changed
}

/// 配置里这个字段现在的明文（不改配置）。
fn plain_of(settings: &Settings, field: &SecretField) -> Option<String> {
    let mut copy = Settings {
        model_configs: settings.model_configs.clone(),
        engine_options: settings.engine_options.clone(),
        ..Settings::default()
    };
    take_plain(&mut copy, field).1
}

/// 保存设置时（界面的通用保存、`settings.update`）：补丁里写进来的 Key 存进凭据库（覆盖），配置里不留；
/// 空字符串当“不改”。存不进凭据库就报错（不改成明文存）。`previous` 是改之前的设置：读配置时没迁走的明文
/// （凭据库当时打不开）这次没改，就原样留着，不因为它让这次保存失败（下次启动再迁）。
pub fn take(
    settings: &mut Settings,
    previous: &Settings,
    store: &dyn SecretStore,
) -> Result<(), String> {
    for field in FIELDS {
        let before = plain_of(previous, field);
        let now = plain_of(settings, field);
        if now.is_some() && now == before {
            continue;
        }
        let (found, plain) = take_plain(settings, field);
        if !found {
            continue;
        }
        if let Some(plain) = plain {
            store.set(&field.name(), &plain).map_err(|error| {
                format!(
                    "{} 的 Key 没能存进 Windows 的安全存储：{error}",
                    field.model
                )
            })?;
        }
    }
    Ok(())
}

/// 会话配置（`options`）里补上 Key：这些模型 ID 里有 Key 字段、配置里没有、凭据库里有，就填进去。
pub fn fill<'a>(
    options: &mut Value,
    models: impl IntoIterator<Item = &'a str>,
    store: &dyn SecretStore,
) {
    let Some(map) = options.as_object_mut() else {
        return;
    };
    for model in models {
        let Some(field) = field_of(model) else {
            continue;
        };
        if text(map.get(field.field)).is_some() {
            continue;
        }
        if let Some(key) = get_from(store, model) {
            map.insert(field.field.into(), Value::String(key));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    #[derive(Default)]
    struct Memory {
        inner: Mutex<BTreeMap<String, String>>,
        broken: bool,
    }

    impl SecretStore for Memory {
        fn get(&self, id: &str) -> Result<Option<String>, String> {
            if self.broken {
                return Err("凭据库打不开".into());
            }
            Ok(self.inner.lock().unwrap().get(id).cloned())
        }
        fn set(&self, id: &str, secret: &str) -> Result<(), String> {
            if self.broken {
                return Err("凭据库打不开".into());
            }
            self.inner
                .lock()
                .unwrap()
                .insert(id.to_string(), secret.to_string());
            Ok(())
        }
        fn delete(&self, id: &str) -> Result<(), String> {
            self.inner.lock().unwrap().remove(id);
            Ok(())
        }
    }

    fn with_plain() -> Settings {
        let mut s = Settings::default();
        s.model_configs.insert(
            "volcano-asr".into(),
            serde_json::json!({"appId": "app-1", "accessToken": "plain-token"}),
        );
        s.model_configs.insert(
            "iflytek-asr".into(),
            serde_json::json!({"appId": "x", "apiKey": "k", "apiSecret": "plain-secret"}),
        );
        s
    }

    #[test]
    fn names_follow_the_category_and_model_scheme() {
        assert_eq!(name_of("volcano-asr").unwrap(), "asr:volcano-asr");
        assert_eq!(name_of("minimax-tts").unwrap(), "tts:minimax-tts");
        assert!(name_of("sherpa-onnx-sensevoice").is_none());
        // 一个模型最多一个 Key 字段
        for field in FIELDS {
            assert_eq!(FIELDS.iter().filter(|f| f.model == field.model).count(), 1);
        }
    }

    #[test]
    fn plaintext_moves_into_the_store_once_and_leaves_the_config() {
        let store = Memory::default();
        let mut s = with_plain();
        assert!(migrate(&mut s, &store));
        assert_eq!(
            store.get("asr:volcano-asr").unwrap().as_deref(),
            Some("plain-token")
        );
        assert_eq!(
            store.get("asr:iflytek-asr").unwrap().as_deref(),
            Some("plain-secret")
        );
        let json = serde_json::to_string(&s).unwrap();
        assert!(!json.contains("plain-token") && !json.contains("plain-secret"));
        // 别的字段不动
        assert_eq!(s.model_configs["volcano-asr"]["appId"], "app-1");
        assert_eq!(s.model_configs["iflytek-asr"]["apiKey"], "k");
        // 第二次没有可迁的
        assert!(!migrate(&mut s, &store));
    }

    #[test]
    fn migration_keeps_an_existing_store_value_and_keeps_plaintext_when_the_store_fails() {
        let store = Memory::default();
        store.set("asr:volcano-asr", "newer").unwrap();
        let mut s = with_plain();
        migrate(&mut s, &store);
        assert_eq!(
            store.get("asr:volcano-asr").unwrap().as_deref(),
            Some("newer")
        );
        assert!(s.model_configs["volcano-asr"].get("accessToken").is_none());

        let broken = Memory {
            broken: true,
            ..Memory::default()
        };
        let mut s = with_plain();
        assert!(!migrate(&mut s, &broken));
        assert_eq!(s.model_configs["volcano-asr"]["accessToken"], "plain-token");
    }

    #[test]
    fn older_engine_options_are_migrated_too() {
        let store = Memory::default();
        let mut s = Settings {
            engine_options: serde_json::json!({"online-asr": {"accessToken": "old", "model": "volcano-asr"}}),
            ..Settings::default()
        };
        assert!(migrate(&mut s, &store));
        assert_eq!(
            store.get("asr:volcano-asr").unwrap().as_deref(),
            Some("old")
        );
        assert!(s.engine_options["online-asr"].get("accessToken").is_none());
        assert_eq!(s.engine_options["online-asr"]["model"], "volcano-asr");
    }

    #[test]
    fn saving_overwrites_the_store_and_an_empty_value_means_unchanged() {
        let store = Memory::default();
        store.set("asr:volcano-asr", "old").unwrap();
        let mut s = Settings::default();
        s.model_configs.insert(
            "volcano-asr".into(),
            serde_json::json!({"appId": "a", "accessToken": "typed"}),
        );
        let before = Settings::default();
        take(&mut s, &before, &store).unwrap();
        assert_eq!(
            store.get("asr:volcano-asr").unwrap().as_deref(),
            Some("typed")
        );
        assert!(s.model_configs["volcano-asr"].get("accessToken").is_none());
        s.model_configs.insert(
            "volcano-asr".into(),
            serde_json::json!({"appId": "a", "accessToken": ""}),
        );
        take(&mut s, &before, &store).unwrap();
        assert_eq!(
            store.get("asr:volcano-asr").unwrap().as_deref(),
            Some("typed")
        );
        assert!(s.model_configs["volcano-asr"].get("accessToken").is_none());
        // 凭据库打不开：新填的 Key 报错（不改成明文存），错误里没有 Key
        let broken = Memory {
            broken: true,
            ..Memory::default()
        };
        let mut s = with_plain();
        let error = take(&mut s, &before, &broken).unwrap_err();
        assert!(!error.contains("plain-token"), "{error}");
        // 读配置时没迁走的旧明文，这次保存没动它：不让保存失败，原样留着等下次启动再迁
        let previous = with_plain();
        let mut s = with_plain();
        s.model_configs
            .insert("other".into(), serde_json::json!({"x": 1}));
        take(&mut s, &previous, &broken).unwrap();
        assert_eq!(s.model_configs["volcano-asr"]["accessToken"], "plain-token");
    }

    #[test]
    fn session_options_get_the_key_from_the_store() {
        let store = Memory::default();
        store.set("asr:volcano-asr", "stored").unwrap();
        let mut options = serde_json::json!({"appId": "a", "model": "volcano-asr"});
        fill(&mut options, ["volcano-asr"], &store);
        assert_eq!(options["accessToken"], "stored");
        // 配置里还有（没迁走的）就用配置里的
        let mut options = serde_json::json!({"accessToken": "plain"});
        fill(&mut options, ["volcano-asr"], &store);
        assert_eq!(options["accessToken"], "plain");
    }
}
