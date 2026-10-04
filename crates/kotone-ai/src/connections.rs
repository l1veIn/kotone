//! 连接目录的增删改：公开字段进 Settings，密钥进 SecretStore。

use kotone_core::connection::{Connection, SecretStore};
use kotone_core::settings::{Settings, SettingsRepository};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionInfo {
    pub id: String,
    pub display_name: String,
    pub kind: kotone_core::connection::ConnectionKind,
    pub provider: String,
    pub base_url: String,
    pub model: String,
    pub has_api_key: bool,
}

impl ConnectionInfo {
    pub fn from_connection(connection: &Connection, has_api_key: bool) -> Self {
        Self {
            id: connection.id.clone(),
            display_name: connection.display_name.clone(),
            kind: connection.kind,
            provider: connection.provider.clone(),
            base_url: connection.base_url.clone(),
            model: connection.model.clone(),
            has_api_key,
        }
    }
}

pub fn list_connection_info(
    settings: &Settings,
    secrets: &dyn SecretStore,
) -> Result<Vec<ConnectionInfo>, String> {
    settings
        .connections
        .iter()
        .map(|connection| {
            let has_api_key = secrets.get(&connection.id)?.is_some();
            Ok(ConnectionInfo::from_connection(connection, has_api_key))
        })
        .collect()
}

pub fn upsert_connection(
    repository: &SettingsRepository,
    secrets: &dyn SecretStore,
    connection: Connection,
    api_key: Option<String>,
) -> Result<Settings, String> {
    connection.validate()?;
    let key = api_key
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let id = connection.id.clone();
    // Capture and compensate the credential under the repository's transaction lock.
    // A failed disk write must never change a key, and a credential backend may
    // report failure after writing (for example, its read-back verification failed).
    let previous_key = std::cell::RefCell::new(None);
    repository
        .update_with(
            |next| {
                let existed = next.connections.iter().any(|item| item.id == id);
                if !existed && key.is_none() {
                    return Err("新建连接必须填写 API key".into());
                }
                if key.is_some() {
                    *previous_key.borrow_mut() = secrets.get(&id)?;
                }
                upsert_connection_record(&mut next.connections, connection.clone())
            },
            |_, _| match key.as_deref() {
                Some(secret) => secrets.set(&id, secret),
                None => Ok(()),
            },
            |_, _| restore_key(secrets, &id, previous_key.borrow().as_deref()),
        )
        .map(|(_, next, ())| next)
}

pub fn delete_connection(
    repository: &SettingsRepository,
    secrets: &dyn SecretStore,
    connection_id: &str,
) -> Result<Settings, String> {
    let id = connection_id.trim();
    if id.is_empty() {
        return Err("连接 ID 不能为空".into());
    }
    let previous_key = std::cell::RefCell::new(None);
    repository
        .update_with(
            |next| {
                let before = next.connections.len();
                next.connections.retain(|connection| connection.id != id);
                if next.connections.len() == before {
                    return Err(format!("未找到连接：{id}"));
                }
                *previous_key.borrow_mut() = secrets.get(id)?;
                Ok(())
            },
            |_, _| secrets.delete(id),
            |_, _| restore_key(secrets, id, previous_key.borrow().as_deref()),
        )
        .map(|(_, next, ())| next)
}

fn restore_key(secrets: &dyn SecretStore, id: &str, previous: Option<&str>) -> Result<(), String> {
    match previous {
        Some(key) => secrets.set(id, key),
        None => secrets.delete(id),
    }
}

fn upsert_connection_record(
    connections: &mut Vec<Connection>,
    connection: Connection,
) -> Result<(), String> {
    if let Some(existing) = connections.iter_mut().find(|item| item.id == connection.id) {
        *existing = connection;
        return Ok(());
    }
    connections.push(connection);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, RwLock};

    use kotone_core::connection::ConnectionKind;
    use kotone_core::settings::Settings;

    use crate::secrets::MemorySecretStore;

    fn sample(id: &str) -> Connection {
        Connection {
            id: id.into(),
            display_name: "通义".into(),
            kind: ConnectionKind::Remote,
            provider: "dashscope".into(),
            base_url: "https://example.test/v1".into(),
            model: "qwen-turbo".into(),
        }
    }

    #[test]
    fn upsert_requires_key_for_new_connection_and_keeps_old_key_on_edit() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let settings = Settings::default();
        kotone_core::settings::save_to(&path, &settings).unwrap();
        let repository = SettingsRepository::new_at(Arc::new(RwLock::new(settings)), path);
        let secrets = MemorySecretStore::default();

        let error = upsert_connection(&repository, &secrets, sample("ds"), None).unwrap_err();
        assert!(error.contains("必须填写"));

        upsert_connection(&repository, &secrets, sample("ds"), Some(" sk-one ".into())).unwrap();
        assert_eq!(secrets.get("ds").unwrap().as_deref(), Some("sk-one"));

        let mut renamed = sample("ds");
        renamed.display_name = "通义主号".into();
        upsert_connection(&repository, &secrets, renamed, None).unwrap();
        assert_eq!(secrets.get("ds").unwrap().as_deref(), Some("sk-one"));
        assert_eq!(
            repository.snapshot().connections[0].display_name,
            "通义主号"
        );
    }

    #[test]
    fn delete_removes_record_and_secret() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let settings = Settings::default();
        kotone_core::settings::save_to(&path, &settings).unwrap();
        let repository = SettingsRepository::new_at(Arc::new(RwLock::new(settings)), path);
        let secrets = MemorySecretStore::default();
        upsert_connection(&repository, &secrets, sample("ds"), Some("sk".into())).unwrap();
        delete_connection(&repository, &secrets, "ds").unwrap();
        assert!(repository.snapshot().connections.is_empty());
        assert!(secrets.get("ds").unwrap().is_none());
    }

    #[test]
    fn failed_disk_write_does_not_replace_existing_key() {
        let dir = tempfile::tempdir().unwrap();
        let blocker = dir.path().join("blocked");
        std::fs::write(&blocker, "not a directory").unwrap();
        let mut settings = Settings::default();
        settings.connections.push(sample("ds"));
        let repository = SettingsRepository::new_at(
            Arc::new(RwLock::new(settings)),
            blocker.join("config.json"),
        );
        let secrets = MemorySecretStore::default();
        secrets.set("ds", "old-key").unwrap();
        let mut edited = sample("ds");
        edited.display_name = "new name".into();
        assert!(upsert_connection(&repository, &secrets, edited, Some("new-key".into())).is_err());
        assert_eq!(secrets.get("ds").unwrap().as_deref(), Some("old-key"));
        assert_eq!(repository.snapshot().connections[0].display_name, "通义");
    }

    struct FailingStore {
        inner: MemorySecretStore,
        fail_write: std::sync::atomic::AtomicBool,
        fail_delete: bool,
    }

    impl SecretStore for FailingStore {
        fn get(&self, id: &str) -> Result<Option<String>, String> {
            self.inner.get(id)
        }
        fn set(&self, id: &str, key: &str) -> Result<(), String> {
            self.inner.set(id, key)?;
            if self
                .fail_write
                .swap(false, std::sync::atomic::Ordering::SeqCst)
            {
                Err("read-back failed".into())
            } else {
                Ok(())
            }
        }
        fn delete(&self, id: &str) -> Result<(), String> {
            self.inner.delete(id)?;
            if self.fail_delete {
                Err("delete verification failed".into())
            } else {
                Ok(())
            }
        }
    }

    #[test]
    fn credential_failures_restore_metadata_and_previous_key() {
        for delete in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("config.json");
            let mut settings = Settings::default();
            settings.connections.push(sample("ds"));
            kotone_core::settings::save_to(&path, &settings).unwrap();
            let repository =
                SettingsRepository::new_at(Arc::new(RwLock::new(settings)), path.clone());
            let secrets = FailingStore {
                inner: MemorySecretStore::default(),
                fail_write: std::sync::atomic::AtomicBool::new(!delete),
                fail_delete: delete,
            };
            secrets.inner.set("ds", "old-key").unwrap();
            let result = if delete {
                delete_connection(&repository, &secrets, "ds")
            } else {
                let mut edited = sample("ds");
                edited.display_name = "new name".into();
                upsert_connection(&repository, &secrets, edited, Some("new-key".into()))
            };
            assert!(result.is_err());
            assert_eq!(repository.snapshot().connections, vec![sample("ds")]);
            assert_eq!(
                kotone_core::settings::load_from(&path).connections,
                vec![sample("ds")]
            );
            assert_eq!(secrets.get("ds").unwrap().as_deref(), Some("old-key"));
        }
    }
}
