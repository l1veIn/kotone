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
    let existed = repository
        .snapshot()
        .connections
        .iter()
        .any(|item| item.id == id);

    if !existed && key.is_none() {
        return Err("新建连接必须填写 API key".into());
    }

    if let Some(secret) = key.as_deref() {
        secrets.set(&id, secret)?;
    }

    match repository.update(|next| {
        upsert_connection_record(&mut next.connections, connection.clone())
    }) {
        Ok(settings) => Ok(settings),
        Err(error) => {
            if !existed {
                let _ = secrets.delete(&id);
            }
            Err(error)
        }
    }
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
    let settings = repository.update(|next| {
        let before = next.connections.len();
        next.connections.retain(|connection| connection.id != id);
        if next.connections.len() == before {
            return Err(format!("未找到连接：{id}"));
        }
        Ok(())
    })?;
    secrets.delete(id)?;
    Ok(settings)
}

fn upsert_connection_record(
    connections: &mut Vec<Connection>,
    connection: Connection,
) -> Result<(), String> {
    if let Some(existing) = connections
        .iter_mut()
        .find(|item| item.id == connection.id)
    {
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

        upsert_connection(
            &repository,
            &secrets,
            sample("ds"),
            Some(" sk-one ".into()),
        )
        .unwrap();
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
}
