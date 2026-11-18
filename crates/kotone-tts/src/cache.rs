//! 合成结果缓存：按“声音（含提供方、模型与端点）+ 语速 + 文本”缓存解码后的 PCM。
//!
//! 快捷消息的固定回答、反复出现的提醒语第二次起不再推理或请求在线接口。只在内存里，
//! 进程退出即清空；按最近使用淘汰，条目数与总样本数都有上限。密钥不进缓存键。
//! Windows 系统语音由系统直接出声、不经过 PCM，不缓存。

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
