//! 连接 API key：系统凭据库，不进 config.json。

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, RwLock};

use keyring::Entry;
use kotone_core::connection::{ConnectionResolver, ResolvedConnection, SecretStore};
use kotone_core::settings::{Settings, SettingsConnectionResolver};

const SERVICE: &str = "kotone";

/// 凭据库服务名。设置了 `KOTONE_HOME`（隔离的数据目录：自动化测试、Agent 沙箱）时按该目录
/// 区分（`kotone@<目录>`），隔离实例既读不到也写不进用户主实例的 API key；未设置时为
/// `kotone`（与旧版本一致，已保存的 key 不受影响）。
fn service_name() -> String {
    service_for(std::env::var_os(kotone_core::settings::HOME_ENV))
}

fn service_for(home: Option<std::ffi::OsString>) -> String {
    match home {
        Some(dir) if !dir.is_empty() => format!("{SERVICE}@{}", dir.to_string_lossy()),
        _ => SERVICE.to_string(),
    }
}

fn credential_user(connection_id: &str) -> String {
    format!("connection:{connection_id}")
}

pub struct KeyringSecretStore;

impl SecretStore for KeyringSecretStore {
    fn get(&self, connection_id: &str) -> Result<Option<String>, String> {
        let entry = Entry::new(&service_name(), &credential_user(connection_id))
            .map_err(|error| format!("打开 Key 的存储失败：{error}"))?;
        match entry.get_password() {
            Ok(secret) if !secret.is_empty() => Ok(Some(secret)),
            Ok(_) => Ok(None),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(format!("读取 Key 失败：{error}")),
        }
    }

    fn set(&self, connection_id: &str, secret: &str) -> Result<(), String> {
        let entry = Entry::new(&service_name(), &credential_user(connection_id))
            .map_err(|error| format!("打开 Key 的存储失败：{error}"))?;
        entry
            .set_password(secret)
            .map_err(|error| format!("保存 Key 失败：{error}"))?;
        match self.get(connection_id)? {
            Some(stored) if stored == secret => Ok(()),
            Some(_) => Err("Key 已保存，但读回来的内容不一致".into()),
            None => Err(
                "Key 没能存进 Windows 的安全存储（读回来是空的）。请确认 Kotone 有权限访问 Windows 凭据管理器。"
                    .into(),
            ),
        }
    }

    fn delete(&self, connection_id: &str) -> Result<(), String> {
        let entry = Entry::new(&service_name(), &credential_user(connection_id))
            .map_err(|error| format!("打开 Key 的存储失败：{error}"))?;
        match entry.delete_credential() {
            Ok(()) => Ok(()),
            Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(format!("删除 Key 失败：{error}")),
        }
    }
}

#[derive(Default)]
pub struct MemorySecretStore {
    inner: Mutex<BTreeMap<String, String>>,
}

impl SecretStore for MemorySecretStore {
    fn get(&self, connection_id: &str) -> Result<Option<String>, String> {
        Ok(self.inner.lock().unwrap().get(connection_id).cloned())
    }

    fn set(&self, connection_id: &str, secret: &str) -> Result<(), String> {
        self.inner
            .lock()
            .unwrap()
            .insert(connection_id.to_string(), secret.to_string());
        Ok(())
    }

    fn delete(&self, connection_id: &str) -> Result<(), String> {
        self.inner.lock().unwrap().remove(connection_id);
        Ok(())
    }
}

/// 公开字段走 Settings，密钥走 SecretStore。
pub struct SecretBackedResolver {
    inner: SettingsConnectionResolver,
    secrets: Arc<dyn SecretStore>,
}

impl SecretBackedResolver {
    pub fn new(settings: Arc<RwLock<Settings>>, secrets: Arc<dyn SecretStore>) -> Self {
        Self {
            inner: SettingsConnectionResolver::new(settings),
            secrets,
        }
    }
}

impl ConnectionResolver for SecretBackedResolver {
    fn resolve(&self, connection_id: &str) -> Result<ResolvedConnection, String> {
        let mut resolved = self.inner.resolve(connection_id)?;
        resolved.api_key = self.secrets.get(connection_id)?;
        Ok(resolved)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kotone_core::connection::{Connection, ConnectionKind};

    #[test]
    fn secret_backed_resolver_fills_key_from_store() {
        let mut settings = Settings::default();
        settings.connections.push(Connection {
            id: "ds".into(),
            display_name: "通义".into(),
            kind: ConnectionKind::Remote,
            provider: "dashscope".into(),
            base_url: "https://example.test/v1".into(),
            model: "qwen-turbo".into(),
        });
        let secrets = Arc::new(MemorySecretStore::default());
        secrets.set("ds", "sk-test").unwrap();
        let resolver = SecretBackedResolver::new(Arc::new(RwLock::new(settings)), secrets);
        let resolved = resolver.resolve("ds").unwrap();
        assert_eq!(resolved.api_key.as_deref(), Some("sk-test"));
    }
}

#[cfg(test)]
mod service_name_tests {
    #[test]
    fn isolated_homes_get_their_own_credential_service() {
        assert_eq!(super::service_for(None), "kotone");
        assert_eq!(super::service_for(Some("".into())), "kotone");
        assert_eq!(
            super::service_for(Some(r"D:\work\kotone-home".into())),
            r"kotone@D:\work\kotone-home"
        );
    }
}
