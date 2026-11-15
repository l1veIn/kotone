//! kotone-tools：面向 Agent 的本机工具服务（M1 视觉输入 + M2 预处理/OCR/聊天跟踪）。
//!
//! - `protocol`：v1 请求/结果信封、错误码、计时
//! - `catalog`：已实现工具与 JSON Schema（只列真实可调用的工具）
//! - `image` / `artifacts`：不可变 RGBA 图像、裁剪校验、PNG 编码与有界产物存储
//! - `preprocess`：显式预设/操作的图像预处理与坐标比例
//! - `ocr_results` / `tracking`：驻留 OCR 结果与聊天消息跟踪（会话、代际、游标）
//! - `service`：调度、并发/期限、窗口引用与产物提交
//! - `server`：127.0.0.1 随机端口 HTTP 服务，每个端点校验会话凭证
//! - `discovery`：当前用户/会话私有的描述文件发布与校验
//! - `client`：CLI 等客户端直接连接正在运行的桌面实例
//!
//! 窗口观察、捕获与私有文件能力通过 `kotone_core::tools` 端口注入；本 crate 不依赖
//! Windows API 或 Tauri。权威手册见 docs/tool-service.md。

pub mod artifacts;
pub mod catalog;
pub mod client;
pub mod discovery;
pub mod ids;
pub mod image;
pub mod limits;
pub mod ocr_results;
pub mod preprocess;
pub mod protocol;
pub mod server;
pub mod service;
#[cfg(any(test, feature = "test-support"))]
#[doc(hidden)]
pub mod testing;
#[cfg(test)]
mod tests_http;
#[cfg(test)]
mod tests_m2;
#[cfg(test)]
mod tests_races;
pub mod tracking;

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use kotone_core::tools::ToolHost;

pub use client::ToolClient;
pub use limits::ServiceLimits;
pub use service::{AsrPhaseProvider, ServiceDeps, ToolService};

/// 由桌面生命周期持有的运行中服务。
pub struct RunningToolService {
    service: Arc<ToolService>,
    server: Option<server::ServerHandle>,
    publication: Option<discovery::Publication>,
    maintenance: Option<tokio::task::JoinHandle<()>>,
    addr: SocketAddr,
}

impl RunningToolService {
    /// 启动 HTTP 服务并发布描述文件。发布失败时服务随之关闭。
    pub async fn start(
        deps: ServiceDeps,
        host: Arc<dyn ToolHost>,
        dir: &Path,
    ) -> Result<Self, String> {
        let token = ids::SecretToken::generate();
        let app_version = deps.app_version.clone();
        let created = deps.clock.now_unix_ms();
        let service = ToolService::new(deps);
        let server = server::serve(service.clone(), token.clone())
            .await
            .map_err(|e| format!("绑定 127.0.0.1 失败: {e}"))?;
        let addr = server.local_addr();
        let pid = std::process::id();
        let descriptor = discovery::Descriptor {
            schema: discovery::DESCRIPTOR_SCHEMA.into(),
            protocol: protocol::PROTOCOL.into(),
            protocol_version: protocol::PROTOCOL_VERSION,
            instance_id: service.instance_id().to_string(),
            app_version,
            pid,
            process_started_at: host.process_started_at(pid),
            session_id: host.session_id(),
            address: addr.to_string(),
            token: token.expose().to_string(),
            created_at_unix_ms: created,
        };
        let publication = match discovery::publish(dir, host, &descriptor) {
            Ok(publication) => publication,
            Err(error) => {
                server.shutdown().await;
                return Err(error.to_string());
            }
        };
        let maintenance = service.spawn_maintenance();
        // 后台做一次 OCR 后端就绪检查（模型哈希、凭据读取），不阻塞启动与异步线程
        let refresh = service.clone();
        tokio::task::spawn_blocking(move || {
            refresh.refresh_ocr_readiness();
        });
        Ok(Self {
            service,
            server: Some(server),
            publication: Some(publication),
            maintenance: Some(maintenance),
            addr,
        })
    }

    pub fn instance_id(&self) -> &str {
        self.service.instance_id()
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn descriptor_path(&self) -> Option<PathBuf> {
        self.publication.as_ref().map(|p| p.path().to_path_buf())
    }

    pub fn service(&self) -> &Arc<ToolService> {
        &self.service
    }

    /// 同步关闭（应用退出路径）。顺序：先在存储锁内撤销准入/提交并丢弃产物（之后任何
    /// 在途请求都无法再发布产物或执行释放），再删除本实例描述文件，最后停止 HTTP 与维护任务。
    pub fn shutdown_now(&mut self) {
        self.service.close();
        if let Some(publication) = self.publication.take() {
            publication.remove_if_owned();
        }
        if let Some(server) = self.server.take() {
            server.signal_stop();
            drop(server);
        }
        if let Some(maintenance) = self.maintenance.take() {
            maintenance.abort();
        }
    }

    /// 异步关闭：顺序同 [`Self::shutdown_now`]，并等待接受循环与维护任务退出。
    pub async fn shutdown(mut self) {
        self.service.close();
        if let Some(publication) = self.publication.take() {
            publication.remove_if_owned();
        }
        if let Some(server) = self.server.take() {
            server.shutdown().await;
        }
        if let Some(maintenance) = self.maintenance.take() {
            let _ = maintenance.await;
        }
    }
}

impl Drop for RunningToolService {
    fn drop(&mut self) {
        self.shutdown_now();
    }
}
