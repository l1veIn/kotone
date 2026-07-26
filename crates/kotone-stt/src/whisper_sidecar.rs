//! 引擎 #1：whisper.cpp sidecar 子进程（ADR-003）。
//!
//! 非流式：push_audio 只缓冲 f32 PCM（16kHz mono），finalize 时：
//! 1. PCM 写 16kHz 16bit WAV 临时文件（~/.kotone/tmp/）；
//! 2. 拉起 whisper-cli（`-np -nt`：stdout 只出纯文本；`--prompt` 注入热词）；
//! 3. 解析 stdout → SttEvent::Final（latency_ms = 转写耗时）。
//!
//! 进程治理：单次转写 30s 硬超时（到点 kill 子进程）；session cancel 立即 kill；
//! Windows 下 CREATE_NO_WINDOW 避免子进程控制台窗口闪现。

use std::io::Write;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::sync::mpsc;

use kotone_core::stt::{EngineCapabilities, SessionConfig, SttEngine, SttEvent, SttSession, Transcript};

use crate::model;

/// 单次转写硬超时（进程级兜底；orchestrator 层另有 finalize 超时）
const TRANSCRIBE_TIMEOUT: Duration = Duration::from_secs(30);

pub struct WhisperSidecarEngine;

impl SttEngine for WhisperSidecarEngine {
    fn id(&self) -> &'static str {
        "whisper-cpp-sidecar"
    }

    fn display_name(&self) -> &str {
        "whisper.cpp (sidecar)"
    }

    fn capabilities(&self) -> EngineCapabilities {
        EngineCapabilities {
            streaming: false,
            hotwords: true, // --prompt 热词注入
            gpu: false,
            offline: true,
            languages: vec!["zh".into(), "en".into()],
        }
    }

    fn is_ready(&self) -> bool {
        if !model::bin_installed() {
            return false;
        }
        let model_id = model::active_model(self.id());
        model::model_path(&model_id).is_some_and(|p| p.exists())
    }

    /// 预热：sidecar 无驻留资源（每次识别才 spawn 子进程），warmup 做就绪检查，
    /// 失败信息对齐 start_session 的引导文案
    fn warmup(&self) -> Result<(), String> {
        if !model::bin_installed() {
            return Err(format!(
                "whisper-cli 未安装（{}）。请在设置页下载，或运行 kotone-cli download bin",
                model::whisper_cli_path().display()
            ));
        }
        let model_id = model::active_model(self.id());
        if model::model_path(&model_id).filter(|p| p.exists()).is_none() {
            return Err(format!(
                "模型 {model_id} 未下载。请在设置页下载，或运行 kotone-cli download {}",
                model_id.trim_start_matches("ggml-")
            ));
        }
        Ok(())
    }

    fn start_session(
        &self,
        cfg: &SessionConfig,
        events: mpsc::UnboundedSender<SttEvent>,
    ) -> Result<Box<dyn SttSession>, String> {
        if !model::bin_installed() {
            return Err(format!(
                "whisper-cli 未安装（{}）。请在设置页下载，或运行 kotone-cli download bin",
                model::whisper_cli_path().display()
            ));
        }
        let model_id = active_or_configured_model(cfg);
        let model_path = model::model_path(&model_id)
            .filter(|p| p.exists())
            .ok_or_else(|| {
                format!(
                    "模型 {model_id} 未下载。请在设置页下载，或运行 kotone-cli download {}",
                    model_id.trim_start_matches("ggml-")
                )
            })?;

        Ok(Box::new(WhisperSession {
            cfg: cfg.clone(),
            events,
            pcm: Vec::new(),
            cancelled: false,
            model_path,
            child: Arc::new(Mutex::new(None)),
        }))
    }
}

/// cfg.options["model"] 优先，否则读 config.json 的活动模型
fn active_or_configured_model(cfg: &SessionConfig) -> String {
    cfg.options
        .get("model")
        .and_then(|m| m.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| model::active_model("whisper-cpp-sidecar"))
}

struct WhisperSession {
    cfg: SessionConfig,
    events: mpsc::UnboundedSender<SttEvent>,
    /// 缓冲的全部 PCM（16kHz mono f32）
    pcm: Vec<f32>,
    cancelled: bool,
    model_path: PathBuf,
    /// 转写中的子进程（cancel 时 kill）
    child: Arc<Mutex<Option<Child>>>,
}

impl WhisperSession {
    fn threads(&self) -> u32 {
        self.cfg
            .options
            .get("threads")
            .and_then(|t| t.as_u64())
            .map(|t| t.clamp(1, 32) as u32)
            .unwrap_or(4)
    }
}

impl SttSession for WhisperSession {
    fn push_audio(&mut self, pcm: &[f32]) -> Result<(), String> {
        if self.cancelled {
            return Err("会话已取消".into());
        }
        self.pcm.extend_from_slice(pcm);
        Ok(())
    }

    fn finalize(self: Box<Self>) -> Result<Transcript, String> {
        if self.cancelled {
            return Err("会话已取消".into());
        }
        if self.pcm.is_empty() {
            return Err("没有可转写的音频".into());
        }

        let wav = tmp_wav_path();
        if let Some(parent) = wav.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("无法创建临时目录：{e}"))?;
        }
        write_wav(&wav, &self.pcm).map_err(|e| format!("写入临时 wav 失败：{e}"))?;

        let started = Instant::now();
        let result = self.run_whisper(&wav);
        let _ = std::fs::remove_file(&wav);
        let text = result?;
        let latency_ms = started.elapsed().as_millis() as u32;

        let _ = self.events.send(SttEvent::Final {
            text: text.clone(),
            latency_ms,
        });
        Ok(Transcript { text, latency_ms })
    }

    fn cancel(&mut self) {
        self.cancelled = true;
        if let Some(mut child) = self.child.lock().unwrap().take() {
            let _ = child.kill();
            let _ = child.wait(); // 防僵尸
        }
    }
}

impl WhisperSession {
    /// 拉起 whisper-cli 并等待结果（30s 硬超时，超时 kill）
    fn run_whisper(&self, wav: &PathBuf) -> Result<String, String> {
        let prompt = build_prompt(&self.cfg.hotwords);
        let mut cmd = Command::new(model::whisper_cli_path());
        cmd.arg("-m")
            .arg(&self.model_path)
            .arg("-l")
            .arg(&self.cfg.language)
            .arg("-t")
            .arg(self.threads().to_string())
            // -np：除结果外什么都不打印；-nt：不带时间戳 → stdout 即纯文本
            .arg("-np")
            .arg("-nt")
            .arg("--prompt")
            .arg(&prompt)
            .arg(wav)
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        let mut child = cmd
            .spawn()
            .map_err(|e| format!("whisper-cli 启动失败：{e}"))?;
        // 子进程句柄交给 cancel 可 kill 的共享槽；stdout 先取出来放等待线程
        let stdout = child.stdout.take();
        *self.child.lock().unwrap() = Some(child);

        // 等待线程：完整收集输出后通知（wait_with_output 消费 child）
        let child_slot = Arc::clone(&self.child);
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let child = child_slot.lock().unwrap().take();
            let out = match child {
                Some(mut c) => {
                    let mut buf = Vec::new();
                    let read_result = match stdout {
                        Some(mut s) => std::io::Read::read_to_end(&mut s, &mut buf).map(|_| ()),
                        None => Ok(()),
                    };
                    let status = c.wait();
                    match (read_result, status) {
                        (Ok(()), Ok(st)) => Ok((buf, st)),
                        (Err(e), _) => Err(format!("读取 whisper-cli 输出失败：{e}")),
                        (_, Err(e)) => Err(format!("等待 whisper-cli 退出失败：{e}")),
                    }
                }
                None => Err("whisper-cli 进程已消失".into()),
            };
            let _ = tx.send(out);
        });

        match rx.recv_timeout(TRANSCRIBE_TIMEOUT) {
            Ok(Ok((buf, status))) => {
                if !status.success() {
                    return Err(format!("whisper-cli 转写失败（退出码 {:?}）", status.code()));
                }
                let stdout = String::from_utf8_lossy(&buf);
                Ok(parse_output(&stdout))
            }
            Ok(Err(e)) => Err(e),
            Err(_) => {
                // 超时：kill 子进程，避免泄漏
                if let Some(mut c) = self.child.lock().unwrap().take() {
                    let _ = c.kill();
                    let _ = c.wait();
                }
                Err(format!(
                    "转写超时（{}s），已终止 whisper-cli",
                    TRANSCRIBE_TIMEOUT.as_secs()
                ))
            }
        }
    }
}

/// 热词 + 标点引导的 initial prompt（whisper 对 prompt 风格敏感：带标点的 prompt
/// 能显著改善中文输出的标点与断句）
fn build_prompt(hotwords: &[String]) -> String {
    if hotwords.is_empty() {
        "以下是带标点的中文游戏语音。".into()
    } else {
        format!(
            "以下是带标点的中文游戏语音，包含术语：{}。",
            hotwords.join(" ")
        )
    }
}

/// 解析 whisper-cli（-np -nt）stdout：去掉空行与残留的 [时间戳] 行，拼接为单段文本
fn parse_output(stdout: &str) -> String {
    stdout
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .filter(|l| !(l.starts_with('[') && l.contains(']')))
        .collect::<Vec<_>>()
        .join("")
        .trim()
        .to_string()
}

/// 临时 wav 路径（~/.kotone/tmp/，自管理，避免 %TEMP% 权限差异）
fn tmp_wav_path() -> PathBuf {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    kotone_core::settings::kotone_dir()
        .join("tmp")
        .join(format!("stt-{}-{ts}.wav", std::process::id()))
}

/// f32 PCM（-1.0..=1.0）→ 16kHz 16bit mono WAV
fn write_wav(path: &PathBuf, pcm: &[f32]) -> std::io::Result<()> {
    let data_len = (pcm.len() * 2) as u32;
    let mut f = std::io::BufWriter::new(std::fs::File::create(path)?);

    // RIFF header
    f.write_all(b"RIFF")?;
    f.write_all(&(36 + data_len).to_le_bytes())?;
    f.write_all(b"WAVE")?;
    // fmt chunk（PCM, mono, 16kHz, 16bit）
    f.write_all(b"fmt ")?;
    f.write_all(&16u32.to_le_bytes())?;
    f.write_all(&1u16.to_le_bytes())?; // audio format = PCM
    f.write_all(&1u16.to_le_bytes())?; // channels
    f.write_all(&16000u32.to_le_bytes())?; // sample rate
    f.write_all(&32000u32.to_le_bytes())?; // byte rate
    f.write_all(&2u16.to_le_bytes())?; // block align
    f.write_all(&16u16.to_le_bytes())?; // bits per sample
    // data chunk
    f.write_all(b"data")?;
    f.write_all(&data_len.to_le_bytes())?;
    for &s in pcm {
        let v = (s.clamp(-1.0, 1.0) * 32767.0).round() as i16;
        f.write_all(&v.to_le_bytes())?;
    }
    f.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_without_hotwords() {
        assert_eq!(build_prompt(&[]), "以下是带标点的中文游戏语音。");
    }

    #[test]
    fn prompt_with_hotwords() {
        let p = build_prompt(&["闪现".into(), "大龙".into(), "gank".into()]);
        assert!(p.contains("闪现 大龙 gank"), "{p}");
        assert!(p.starts_with("以下是带标点的中文游戏语音"), "{p}");
        assert!(p.ends_with('。'), "{p}");
    }

    #[test]
    fn parse_output_joins_segments_and_trims() {
        // -np -nt 下典型输出：每段一行、首尾空白
        let out = parse_output("  对面打野在下路\n\n");
        assert_eq!(out, "对面打野在下路");
    }

    #[test]
    fn parse_output_drops_timestamp_lines() {
        // 防御：万一 -nt 失效，带时间戳的行应被丢弃
        let out = parse_output("[00:00:00.000 --> 00:00:02.000]  对面打野在下路\n你好\n");
        assert_eq!(out, "你好");
    }

    #[test]
    fn parse_output_empty() {
        assert_eq!(parse_output(""), "");
        assert_eq!(parse_output(" \n \n"), "");
    }

    #[test]
    fn wav_full_structure() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.wav");
        let pcm = [0.5f32, -0.5, 2.0, -2.0];
        write_wav(&p, &pcm).unwrap();
        let b = std::fs::read(&p).unwrap();

        assert_eq!(&b[0..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(b[4..8].try_into().unwrap()), 36 + 8);
        assert_eq!(&b[8..12], b"WAVE");
        assert_eq!(&b[12..16], b"fmt ");
        assert_eq!(u16::from_le_bytes(b[20..22].try_into().unwrap()), 1); // PCM
        assert_eq!(u16::from_le_bytes(b[22..24].try_into().unwrap()), 1); // mono
        assert_eq!(u32::from_le_bytes(b[24..28].try_into().unwrap()), 16000);
        assert_eq!(u16::from_le_bytes(b[34..36].try_into().unwrap()), 16); // bits
        assert_eq!(&b[36..40], b"data");
        assert_eq!(u32::from_le_bytes(b[40..44].try_into().unwrap()), 8);
        let s = |i: usize| i16::from_le_bytes(b[44 + i * 2..46 + i * 2].try_into().unwrap());
        assert_eq!(s(0), 16384); // 0.5 * 32767 ≈ 16384
        assert_eq!(s(1), -16384);
        assert_eq!(s(2), 32767); // 削波
        assert_eq!(s(3), -32767);
    }

    #[test]
    fn engine_not_ready_without_bin() {
        // 测试环境一般没装 ~/.kotone/bin：is_ready 应反映实际（不强断言 false，
        // 只断言调用不 panic；真机 E2E 在 tests/whisper_e2e.rs）
        let e = WhisperSidecarEngine;
        let _ = e.is_ready();
        assert_eq!(e.id(), "whisper-cpp-sidecar");
    }
}
