//! 实例发现：桌面实例把 descriptor 写到当前用户私有的运行目录，客户端读取并校验。
//!
//! 文件名带 Windows 会话 ID：同一用户的另一个登录会话（如远程桌面）看不到、也不会误连
//! 这个实例。描述文件包含会话凭证，因此只能由当前用户访问；任何字段都不打印到日志。

use std::fmt;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use kotone_core::tools::ToolHost;

use crate::ids::SecretToken;
use crate::protocol::{PROTOCOL, PROTOCOL_VERSION};

pub const DESCRIPTOR_SCHEMA: &str = "kotone.tool-service.descriptor/1";
/// 覆盖运行目录（测试与自动化；正常使用无需设置）。
pub const DIR_ENV: &str = "KOTONE_TOOL_SERVICE_DIR";
/// 描述文件大小上限，超过即视为无效。
const MAX_DESCRIPTOR_BYTES: u64 = 16 * 1024;

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Descriptor {
    pub schema: String,
    pub protocol: String,
    pub protocol_version: u32,
    pub instance_id: String,
    pub app_version: String,
    pub pid: u32,
    pub process_started_at: Option<u64>,
    pub session_id: Option<u32>,
    pub address: String,
    pub token: String,
    pub created_at_unix_ms: u64,
}

impl fmt::Debug for Descriptor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Descriptor")
            .field("instance_id", &self.instance_id)
            .field("pid", &self.pid)
            .field("session_id", &self.session_id)
            .field("address", &self.address)
            .field("token", &"<redacted>")
            .finish()
    }
}

impl Descriptor {
    pub fn token(&self) -> SecretToken {
        SecretToken::from_string(self.token.clone())
    }

    pub fn socket_addr(&self) -> Result<SocketAddr, DiscoveryError> {
        let addr: SocketAddr = self
            .address
            .parse()
            .map_err(|_| DiscoveryError::Invalid("address 不是有效的套接字地址".into()))?;
        if !addr.ip().is_loopback() || !addr.is_ipv4() || addr.port() == 0 {
            return Err(DiscoveryError::Invalid(
                "address 必须是 127.0.0.1 上的端口".into(),
            ));
        }
        Ok(addr)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiscoveryError {
    /// 没有描述文件：服务未开启、桌面未运行或已退出。
    NotRunning {
        path: PathBuf,
    },
    /// 描述文件可被其他账户访问或不属于当前用户。
    Insecure(String),
    Invalid(String),
    ProtocolMismatch {
        found: String,
    },
    /// 描述文件指向已退出 / 已更换的进程或其他会话。
    Stale(String),
    /// 另一个仍在运行的实例已经发布了描述文件。
    AlreadyPublished {
        pid: u32,
    },
    Io(String),
}

impl fmt::Display for DiscoveryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotRunning { path } => write!(
                f,
                "未发现运行中的工具服务（{}）。请确认 toolService.enabled=true 并已重启 Kotone 桌面端",
                path.display()
            ),
            Self::Insecure(reason) => write!(f, "描述文件权限不安全，拒绝使用: {reason}"),
            Self::Invalid(reason) => write!(f, "描述文件无效: {reason}"),
            Self::ProtocolMismatch { found } => {
                write!(f, "描述文件协议不兼容（{found}），请升级客户端或桌面端")
            }
            Self::Stale(reason) => write!(f, "描述文件已过期: {reason}"),
            Self::AlreadyPublished { pid } => {
                write!(f, "另一个工具服务实例（PID {pid}）仍在运行")
            }
            Self::Io(reason) => write!(f, "读取描述文件失败: {reason}"),
        }
    }
}

impl std::error::Error for DiscoveryError {}

/// 默认运行目录：`$KOTONE_TOOL_SERVICE_DIR` 或 `~/.kotone/run`。
pub fn default_dir() -> PathBuf {
    match std::env::var_os(DIR_ENV) {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => kotone_core::settings::kotone_dir().join("run"),
    }
}

pub fn descriptor_path(dir: &Path, session_id: Option<u32>) -> PathBuf {
    let session = session_id
        .map(|s| s.to_string())
        .unwrap_or_else(|| "unknown".into());
    dir.join(format!("tool-service-s{session}.json"))
}

fn read_descriptor(host: &dyn ToolHost, path: &Path) -> Result<Descriptor, DiscoveryError> {
    let metadata = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(DiscoveryError::NotRunning {
                path: path.to_path_buf(),
            })
        }
        Err(e) => return Err(DiscoveryError::Io(e.to_string())),
    };
    if !metadata.is_file() || metadata.len() > MAX_DESCRIPTOR_BYTES {
        return Err(DiscoveryError::Invalid("不是普通文件或过大".into()));
    }
    host.verify_private_file(path)
        .map_err(|e| DiscoveryError::Insecure(e.to_string()))?;
    let bytes = std::fs::read(path).map_err(|e| DiscoveryError::Io(e.to_string()))?;
    let raw: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|_| DiscoveryError::Invalid("不是 JSON".into()))?;
    let schema = raw.get("schema").and_then(|s| s.as_str()).unwrap_or("");
    let version = raw.get("protocolVersion").and_then(|s| s.as_u64());
    if schema != DESCRIPTOR_SCHEMA || version != Some(u64::from(PROTOCOL_VERSION)) {
        return Err(DiscoveryError::ProtocolMismatch {
            found: format!("{schema} v{version:?}"),
        });
    }
    serde_json::from_value(raw).map_err(|e| DiscoveryError::Invalid(format!("字段不完整: {e}")))
}

/// 客户端：读取并校验描述文件属于当前用户、当前会话、仍在运行的同一进程。
pub fn load_for_client(dir: &Path, host: &dyn ToolHost) -> Result<Descriptor, DiscoveryError> {
    let session = host.session_id();
    let path = descriptor_path(dir, session);
    let descriptor = read_descriptor(host, &path)?;
    if descriptor.protocol != PROTOCOL {
        return Err(DiscoveryError::ProtocolMismatch {
            found: descriptor.protocol.clone(),
        });
    }
    if descriptor.session_id != session {
        return Err(DiscoveryError::Stale("描述文件属于另一个登录会话".into()));
    }
    descriptor.socket_addr()?;
    if descriptor.token.len() < 32 {
        return Err(DiscoveryError::Invalid("会话凭证格式无效".into()));
    }
    match host.process_started_at(descriptor.pid) {
        None => {
            return Err(DiscoveryError::Stale(format!(
                "发布它的进程（PID {}）已退出",
                descriptor.pid
            )))
        }
        Some(started) => {
            if descriptor.process_started_at.is_some_and(|d| d != started) {
                return Err(DiscoveryError::Stale(format!(
                    "PID {} 已被其他进程复用",
                    descriptor.pid
                )));
            }
        }
    }
    Ok(descriptor)
}

/// 已发布的描述文件。只删除内容仍属于本实例的文件。
pub struct Publication {
    path: PathBuf,
    instance_id: String,
    removed: std::sync::atomic::AtomicBool,
}

impl Publication {
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 删除描述文件（仅当 instanceId 仍是本实例）。返回是否删除。
    pub fn remove_if_owned(&self) -> bool {
        if self.removed.swap(true, std::sync::atomic::Ordering::SeqCst) {
            return false;
        }
        let Ok(bytes) = std::fs::read(&self.path) else {
            return false;
        };
        let owned = serde_json::from_slice::<serde_json::Value>(&bytes)
            .ok()
            .and_then(|v| {
                v.get("instanceId")
                    .and_then(|i| i.as_str())
                    .map(|i| i == self.instance_id)
            })
            .unwrap_or(false);
        owned && std::fs::remove_file(&self.path).is_ok()
    }
}

impl Drop for Publication {
    fn drop(&mut self) {
        let _ = self.remove_if_owned();
    }
}

/// 桌面端：写入描述文件。已有文件若指向仍在运行的另一实例则拒绝覆盖；过期文件被替换。
pub fn publish(
    dir: &Path,
    host: Arc<dyn ToolHost>,
    descriptor: &Descriptor,
) -> Result<Publication, DiscoveryError> {
    host.prepare_private_dir(dir)
        .map_err(|e| DiscoveryError::Io(format!("准备私有运行目录失败: {e}")))?;
    let path = descriptor_path(dir, descriptor.session_id);
    if let Ok(bytes) = std::fs::read(&path) {
        if let Ok(existing) = serde_json::from_slice::<Descriptor>(&bytes) {
            let alive = host
                .process_started_at(existing.pid)
                .is_some_and(|started| existing.process_started_at.map_or(true, |d| d == started));
            if alive && existing.pid != descriptor.pid {
                return Err(DiscoveryError::AlreadyPublished { pid: existing.pid });
            }
        }
    }
    let bytes = serde_json::to_vec_pretty(descriptor)
        .map_err(|e| DiscoveryError::Io(format!("序列化描述文件失败: {e}")))?;
    host.write_private_file(&path, &bytes)
        .map_err(|e| DiscoveryError::Io(format!("写入描述文件失败: {e}")))?;
    Ok(Publication {
        path,
        instance_id: descriptor.instance_id.clone(),
        removed: std::sync::atomic::AtomicBool::new(false),
    })
}
