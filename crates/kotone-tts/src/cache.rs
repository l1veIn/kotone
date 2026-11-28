//! 合成结果缓存：按“声音（含提供方、模型与端点）+ 语速 + 文本”缓存解码后的 PCM。
//!
//! 快捷消息的固定回答、反复出现的提醒语第二次起不再推理或请求在线接口。在内存里，
//! 进程退出即清空；按最近使用淘汰，条目数与总样本数都有上限。密钥不进缓存键。
//! Windows 系统语音由系统直接出声、不经过 PCM，不缓存。
//!
//! 登记过的固定短句（唤醒应答、等待话）另外落盘（[`disk`]），重启后不用重新合成。

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, OnceLock};

use crate::SynthesizedSpeech;

const MAX_ENTRIES: usize = 64;
/// 约 8 分钟 24 kHz 单声道音频（f32 约 46 MB）。
const MAX_SAMPLES: usize = 24_000 * 60 * 8;

#[derive(Default)]
pub struct SynthCache {
    entries: VecDeque<(String, Arc<SynthesizedSpeech>)>,
    samples: usize,
}

impl SynthCache {
    pub fn get(&mut self, key: &str) -> Option<Arc<SynthesizedSpeech>> {
        let index = self.entries.iter().position(|(k, _)| k == key)?;
        let entry = self.entries.remove(index)?;
        let audio = entry.1.clone();
        self.entries.push_back(entry);
        Some(audio)
    }

    pub fn put(&mut self, key: String, audio: Arc<SynthesizedSpeech>) {
        if audio.samples.len() > MAX_SAMPLES {
            return;
        }
        if let Some(index) = self.entries.iter().position(|(k, _)| *k == key) {
            if let Some((_, old)) = self.entries.remove(index) {
                self.samples -= old.samples.len();
            }
        }
        self.samples += audio.samples.len();
        self.entries.push_back((key, audio));
        while self.entries.len() > MAX_ENTRIES || self.samples > MAX_SAMPLES {
            match self.entries.pop_front() {
                Some((_, old)) => self.samples -= old.samples.len(),
                None => break,
            }
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.samples = 0;
    }
}

/// 进程内共享的缓存（Kokoro 与在线引擎共用；键里带提供方，互不混用）。
pub fn global() -> &'static Mutex<SynthCache> {
    static CACHE: OnceLock<Mutex<SynthCache>> = OnceLock::new();
    CACHE.get_or_init(Mutex::default)
}

/// 缓存键（Kokoro 用；在线引擎见 `OnlineSynth::cache_key`）。
pub fn key(voice: &str, rate: i32, text: &str) -> String {
    format!("{voice}\u{1}{rate}\u{1}{text}")
}

/// 固定短句落盘（ADR 029 §四）：唤醒应答“嗯？/我在”、等待话“我看看/稍等哈”合成一次就存成 WAV，
/// 重启后从文件读进内存缓存，不用再请求在线语音、不再花钱，启动后马上能说。
///
/// 照 xiaozhi-esp32-server（MIT）`core/utils/wakeup_word.py`：按音色把唤醒回复存成文件，
/// 换了音色就是另一个文件。这里文件名是缓存键（提供方 + 声音 + 模型 + 端点 + 语速 + 文字，
/// 不含密钥）的 SHA-256；只有 [`configure`] 登记过的话才落盘，模型回答不落盘。
pub mod disk {
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;

    use sha2::{Digest, Sha256};

    use crate::SynthesizedSpeech;

    /// 目录里最多留这么多个文件（换过很多次声音时删最旧的）。
    pub const MAX_FILES: usize = 200;

    struct Config {
        dir: PathBuf,
        texts: Vec<String>,
    }

    static CONFIG: Mutex<Option<Config>> = Mutex::new(None);
    /// 另外登记的话（试听声音的那句，ADR 048）：[`configure`] 替换列表时也留着。
    static EXTRA: Mutex<Vec<String>> = Mutex::new(Vec::new());

    /// 再登记一句要落盘的话（已经登记过就不重复）。落盘目录还是 [`configure`] 给的那个；没配置时只进内存缓存。
    pub fn also(text: &str) {
        let text = text.trim().to_string();
        let mut extra = EXTRA.lock().unwrap();
        if !text.is_empty() && !extra.contains(&text) {
            extra.push(text);
        }
    }

    /// 打开落盘：`texts` 里的话存进 `dir`（再次调用就替换）。
    pub fn configure(dir: impl Into<PathBuf>, texts: &[&str]) {
        *CONFIG.lock().unwrap() = Some(Config {
            dir: dir.into(),
            texts: texts.iter().map(|t| t.trim().to_string()).collect(),
        });
    }

    fn path_for(dir: &Path, key: &str) -> PathBuf {
        let digest = Sha256::digest(key.as_bytes());
        let name: String = digest.iter().take(16).map(|b| format!("{b:02x}")).collect();
        dir.join(format!("{name}.wav"))
    }

    /// 这句登记过落盘：它的文件。
    fn file(key: &str, text: &str) -> Option<PathBuf> {
        let config = CONFIG.lock().unwrap();
        let config = config.as_ref()?;
        // 合成线程送来的文字去掉过挨着中文的空格（`segment::tighten`），登记的话也照样比
        let text = crate::segment::tighten(text.trim());
        let same = |t: &String| crate::segment::tighten(t) == text;
        (config.texts.iter().any(same) || EXTRA.lock().unwrap().iter().any(same))
            .then(|| path_for(&config.dir, key))
    }

    /// 读落盘的这句（没有、没登记或文件坏了返回 None；坏文件删掉）。
    pub fn load(key: &str, text: &str) -> Option<SynthesizedSpeech> {
        let path = file(key, text)?;
        let bytes = std::fs::read(&path).ok()?;
        match crate::wav::decode(&bytes) {
            Ok(speech) => Some(speech),
            Err(error) => {
                kotone_core::log::log(&format!("tts disk cache: dropping {path:?}: {error}"));
                let _ = std::fs::remove_file(&path);
                None
            }
        }
    }

    /// 登记过的话合成好了：存成文件（已经有就不写）。写失败只记日志。
    pub fn store(key: &str, text: &str, speech: &SynthesizedSpeech) {
        let Some(path) = file(key, text) else {
            return;
        };
        if path.exists() || speech.samples.is_empty() {
            return;
        }
        let Some(dir) = path.parent() else {
            return;
        };
        let written = std::fs::create_dir_all(dir).and_then(|_| {
            // 先写临时文件再改名：写到一半退出不会留下半个文件
            let partial = path.with_extension("wav.partial");
            std::fs::write(&partial, crate::wav::encode(speech))?;
            std::fs::rename(&partial, &path)
        });
        match written {
            Ok(()) => prune(dir),
            Err(error) => kotone_core::log::log(&format!("tts disk cache: {error}")),
        }
    }

    /// 文件多于 [`MAX_FILES`] 个时删最旧的。
    fn prune(dir: &Path) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        let mut files: Vec<(std::time::SystemTime, PathBuf)> = entries
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "wav"))
            .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
            .collect();
        if files.len() <= MAX_FILES {
            return;
        }
        files.sort();
        for (_, path) in &files[..files.len() - MAX_FILES] {
            let _ = std::fs::remove_file(path);
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn only_registered_phrases_are_kept_on_disk_and_survive_a_restart() {
            let dir = tempfile::tempdir().unwrap();
            configure(dir.path(), &["我看看", "嗯？"]);
            let speech = SynthesizedSpeech {
                samples: vec![0.25; 2_400],
                sample_rate: 24_000,
            };
            let key = super::super::key("deepseek/stella", 0, "我看看");
            assert!(load(&key, "我看看").is_none());
            store(&key, "我看看", &speech);
            // 模型的回答不落盘
            let answer = super::super::key("deepseek/stella", 0, "你的打野在下路");
            store(&answer, "你的打野在下路", &speech);
            let files = || std::fs::read_dir(dir.path()).unwrap().count();
            assert_eq!(files(), 1);
            // 换了声音是另一个文件
            let other = super::super::key("deepseek/tide", 0, "我看看");
            assert!(load(&other, "我看看").is_none());
            // “重启”：重新登记后照样读得到
            configure(dir.path(), &["我看看"]);
            let back = load(&key, "我看看").unwrap();
            assert_eq!(back.sample_rate, 24_000);
            assert_eq!(back.samples.len(), 2_400);
            // 坏文件删掉、当作没有
            let path = path_for(dir.path(), &key);
            std::fs::write(&path, b"not a wav").unwrap();
            assert!(load(&key, "我看看").is_none());
            assert!(!path.exists());
            // 另外登记的话（试听声音那句）也落盘，重新 configure 也还在
            let line = "你好呀，以后我就用这个声音跟你说话。";
            also(line);
            configure(dir.path(), &["我看看"]);
            let preview = super::super::key("minimax/female-shaonv", 0, line);
            store(&preview, line, &speech);
            assert!(load(&preview, line).is_some());
            // 太多就删最旧的
            for i in 0..MAX_FILES + 5 {
                std::fs::write(dir.path().join(format!("{i:04}.wav")), b"x").unwrap();
            }
            prune(dir.path());
            assert_eq!(files(), MAX_FILES);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn audio(n: usize) -> Arc<SynthesizedSpeech> {
        Arc::new(SynthesizedSpeech {
            samples: vec![0.1; n],
            sample_rate: 24_000,
        })
    }

    #[test]
    fn hits_refresh_recency_and_limits_evict_oldest() {
        let mut cache = SynthCache::default();
        cache.put(key("kokoro/zf_001", 0, "打野有闪了"), audio(10));
        cache.put(key("kokoro/zf_001", 1, "打野有闪了"), audio(10));
        assert!(cache.get(&key("kokoro/zf_001", 0, "打野有闪了")).is_some());
        assert!(cache.get(&key("kokoro/zf_003", 0, "打野有闪了")).is_none());
        for i in 0..MAX_ENTRIES {
            cache.put(format!("k{i}"), audio(1));
        }
        assert_eq!(cache.len(), MAX_ENTRIES);
        assert!(cache.get(&key("kokoro/zf_001", 1, "打野有闪了")).is_none());
        cache.put("huge".into(), audio(MAX_SAMPLES / 2 + 1));
        cache.put("huge2".into(), audio(MAX_SAMPLES / 2 + 1));
        assert!(cache.get("huge").is_none() && cache.get("huge2").is_some());
        cache.clear();
        assert!(cache.is_empty());
    }
}
