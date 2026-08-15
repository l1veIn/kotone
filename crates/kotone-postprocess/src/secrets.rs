//! 连接 API key：系统凭据库，不进 config.json。

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, RwLock};

use keyring::Entry;
use kotone_core::connection::{ConnectionResolver, ResolvedConnection, SecretStore};
use kotone_core::settings::{Settings, SettingsConnectionResolver};

const SERVICE: &str = "kotone";

fn credential_user(connection_id: &str) -> String {
    format!("connection:{connection_id}")
}

pub struct KeyringSecretStore;

impl SecretStore for KeyringSecretStore {
    fn get(&self, connection_id: &str) -> Result<Option<String>, String> {
        let entry = Entry::new(SERVICE, &credential_user(connection_id))
            .map_err(|error| format!("打开凭据项失败：{error}"))?;
        match entry.get_password() {
            Ok(secret) if !secret.is_empty() => Ok(Some(secret)),
            Ok(_) => Ok(None),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(format!("读取凭据失败：{error}")),
        }
    }

    fn set(&self, connection_id: &str, secret: &str) -> Result<(), String> {
        let entry = Entry::new(SERVICE, &credential_user(connection_id))
            .map_err(|error| format!("打开凭据项失败：{error}"))?;
        entry
            .set_password(secret)
            .map_err(|error| format!("写入凭据失败：{error}"))?;
        match self.get(connection_id)? {
            Some(stored) if stored == secret => Ok(()),
            Some(_) => Err("凭据已写入，但回读内容不一致".into()),
            None => Err(
                "凭据未能写入系统凭据库（回读为空）。请确认应用有权限访问 Windows 凭据管理器。"
                    .into(),
            ),
        }
    }

    fn delete(&self, connection_id: &str) -> Result<(), String> {
        let entry = Entry::new(SERVICE, &credential_user(connection_id))
            .map_err(|error| format!("打开凭据项失败：{error}"))?;
        match entry.delete_credential() {
            Ok(()) => Ok(()),
            Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(format!("删除凭据失败：{error}")),
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
