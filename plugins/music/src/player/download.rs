//! 边下边放：播放链接下载到内存里（不落盘，ADR 034 第四节），解码那边按需读；还没下到的位置就等。

use std::io::{self, Read, Seek, SeekFrom};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use symphonia::core::io::MediaSource;

#[derive(Default)]
struct State {
    bytes: Vec<u8>,
    total: Option<u64>,
    done: bool,
    error: Option<String>,
    cancelled: bool,
}

/// 一首歌下载到内存的进度。
#[derive(Default)]
pub(crate) struct Download {
    state: Mutex<State>,
    changed: Condvar,
}

impl Download {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    fn update(&self, f: impl FnOnce(&mut State)) {
        f(&mut self.state.lock().unwrap());
        self.changed.notify_all();
    }

    pub(crate) fn push(&self, chunk: &[u8]) {
        self.update(|s| s.bytes.extend_from_slice(chunk));
    }

    pub(crate) fn set_total(&self, total: Option<u64>) {
        self.update(|s| s.total = total);
    }

    pub(crate) fn finish(&self) {
        self.update(|s| s.done = true);
    }

    pub(crate) fn fail(&self, error: String) {
        self.update(|s| s.error = Some(error));
    }

    /// 停了：读的那边马上返回错误。
    pub(crate) fn cancel(&self) {
        self.update(|s| s.cancelled = true);
    }

    pub(crate) fn cancelled(&self) -> bool {
        self.state.lock().unwrap().cancelled
    }
}

/// 在 tokio 上下载（读一块存一块）。HTTP 出错记成 `HTTP <状态码>`。
pub(crate) async fn fetch(http: reqwest::Client, url: String, download: Arc<Download>) {
    let response = match http.get(&url).send().await {
        Ok(r) => r,
        Err(e) => return download.fail(format!("下载失败：{}", e.without_url())),
    };
    if !response.status().is_success() {
        return download.fail(format!("HTTP {}", response.status().as_u16()));
    }
    download.set_total(response.content_length());
    let mut response = response;
    loop {
        if download.cancelled() {
            return;
        }
        match response.chunk().await {
            Ok(Some(chunk)) => download.push(&chunk),
            Ok(None) => return download.finish(),
            Err(e) => return download.fail(format!("下载中断：{}", e.without_url())),
        }
    }
}

/// 给解码器读的：像文件一样能读能跳，没下到的地方等着。
pub(crate) struct Reader {
    download: Arc<Download>,
    position: u64,
}

impl Reader {
    pub(crate) fn new(download: Arc<Download>) -> Self {
        Self {
            download,
            position: 0,
        }
    }
}

impl Read for Reader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let mut state = self.download.state.lock().unwrap();
        loop {
            if state.cancelled {
                return Err(io::Error::new(io::ErrorKind::Interrupted, "stopped"));
            }
            let len = state.bytes.len() as u64;
            if self.position < len {
                let start = self.position as usize;
                let n = buf.len().min(state.bytes.len() - start);
                buf[..n].copy_from_slice(&state.bytes[start..start + n]);
                self.position += n as u64;
                return Ok(n);
            }
            if state.done {
                return Ok(0);
            }
            if let Some(error) = &state.error {
                return Err(io::Error::other(error.clone()));
            }
            state = self
                .download
                .changed
                .wait_timeout(state, Duration::from_millis(200))
                .unwrap()
                .0;
        }
    }
}

impl Seek for Reader {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        let target = match to {
            SeekFrom::Start(n) => n as i64,
            SeekFrom::Current(d) => self.position as i64 + d,
            SeekFrom::End(d) => {
                // 要知道总长：没给 Content-Length 就等下完。
                let mut state = self.download.state.lock().unwrap();
                let total = loop {
                    if let Some(total) = state.total {
                        break total;
                    }
                    if state.done || state.cancelled || state.error.is_some() {
                        break state.bytes.len() as u64;
                    }
                    state = self
                        .download
                        .changed
                        .wait_timeout(state, Duration::from_millis(200))
                        .unwrap()
                        .0;
                };
                total as i64 + d
            }
        };
        if target < 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "seek before start",
            ));
        }
        self.position = target as u64;
        Ok(self.position)
    }
}

impl MediaSource for Reader {
    fn is_seekable(&self) -> bool {
        true
    }

    fn byte_len(&self) -> Option<u64> {
        self.download.state.lock().unwrap().total
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_wait_for_bytes_and_seek_works() {
        let download = Download::new();
        download.set_total(Some(6));
        download.push(b"abc");
        let mut reader = Reader::new(download.clone());
        let mut buf = [0u8; 8];
        assert_eq!(reader.read(&mut buf).unwrap(), 3);
        let writer = {
            let download = download.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(50));
                download.push(b"def");
                download.finish();
            })
        };
        // 还没下到：等
        assert_eq!(reader.read(&mut buf).unwrap(), 3);
        assert_eq!(&buf[..3], b"def");
        assert_eq!(reader.read(&mut buf).unwrap(), 0, "done");
        writer.join().unwrap();
        assert_eq!(reader.seek(SeekFrom::End(-2)).unwrap(), 4);
        assert_eq!(reader.read(&mut buf).unwrap(), 2);
        assert_eq!(reader.seek(SeekFrom::Start(1)).unwrap(), 1);
        assert_eq!(reader.byte_len(), Some(6));
        download.cancel();
        assert!(reader.read(&mut buf).is_err());
    }

    #[test]
    fn errors_reach_the_reader() {
        let download = Download::new();
        download.fail("HTTP 403".into());
        let mut reader = Reader::new(download);
        let err = reader.read(&mut [0u8; 4]).unwrap_err();
        assert!(err.to_string().contains("403"));
    }
}
