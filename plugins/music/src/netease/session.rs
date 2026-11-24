//! 登录状态：网易云的 Cookie（`MUSIC_U` 是凭证，`__csrf` 防伪）和这台机器固定的设备号。
//! 存进 [`crate::SessionStore`] 的是一段 JSON：`{"cookies":{"MUSIC_U":"…","__csrf":"…"},"deviceId":"…"}`。
//! 设备号照 ncmapi：52 位十六进制，同一台机器一直用同一个（每次都像新设备更容易被风控）。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Session {
    #[serde(default)]
    pub cookies: BTreeMap<String, String>,
    #[serde(default)]
    pub device_id: String,
}

impl Session {
    /// 从存着的那段文字读；读不出来就是新的（设备号补上）。
    pub(crate) fn from_blob(blob: Option<&str>) -> Self {
        let mut session: Session = blob
            .and_then(|b| serde_json::from_str(b).ok())
            .unwrap_or_default();
        if session.device_id.len() != 52 {
            session.device_id = crate::random::chars(b"0123456789ABCDEF", 52);
        }
        session
    }

    pub(crate) fn to_blob(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    pub(crate) fn logged_in(&self) -> bool {
        self.cookies.get("MUSIC_U").is_some_and(|v| !v.is_empty())
    }

    pub(crate) fn cookie(&self, name: &str) -> Option<&str> {
        self.cookies.get(name).map(String::as_str)
    }

    /// 记下一条 `Set-Cookie`（只要名字和值；值为空、`Max-Age=0` 或已过期就删掉）。返回有没有变化。
    pub(crate) fn absorb(&mut self, set_cookie: &str) -> bool {
        let mut parts = set_cookie.split(';');
        let Some((name, value)) = parts.next().and_then(|p| p.split_once('=')) else {
            return false;
        };
        let (name, value) = (name.trim(), value.trim());
        if name.is_empty() {
            return false;
        }
        let deleted = value.is_empty()
            || parts.any(|attr| {
                let attr = attr.trim().to_ascii_lowercase();
                attr == "max-age=0" || attr.starts_with("max-age=-")
            });
        if deleted {
            self.cookies.remove(name).is_some()
        } else if self.cookies.get(name).map(String::as_str) != Some(value) {
            self.cookies.insert(name.to_string(), value.to_string());
            true
        } else {
            false
        }
    }

    /// 退出：Cookie 全丢，设备号留着。
    pub(crate) fn logout(&mut self) {
        self.cookies.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blob_round_trip_keeps_the_device_id() {
        let fresh = Session::from_blob(None);
        assert_eq!(fresh.device_id.len(), 52);
        assert!(!fresh.logged_in());
        let mut s = fresh.clone();
        s.cookies.insert("MUSIC_U".into(), "abc".into());
        let back = Session::from_blob(Some(&s.to_blob()));
        assert_eq!(back, s);
        assert!(back.logged_in());
        assert!(s.to_blob().contains("\"deviceId\""));
        let garbage = Session::from_blob(Some("not json"));
        assert_eq!(garbage.device_id.len(), 52);
    }

    #[test]
    fn set_cookie_headers_add_update_and_delete() {
        let mut s = Session::from_blob(None);
        assert!(s.absorb("MUSIC_U=token; Max-Age=1296000; Expires=Tue, 08 Dec 2026 00:00:00 GMT; Path=/; HTTPOnly"));
        assert!(!s.absorb("MUSIC_U=token; Path=/"), "same value");
        assert!(s.absorb("__csrf=c1; Path=/"));
        assert_eq!(s.cookie("__csrf"), Some("c1"));
        assert!(s.absorb("MUSIC_U=; Max-Age=0; Path=/"));
        assert!(!s.logged_in());
        assert!(s.absorb("__csrf=c2; Max-Age=0"));
        assert_eq!(s.cookie("__csrf"), None);
        assert!(!s.absorb("garbage"));
        s.absorb("NMTID=x");
        let device = s.device_id.clone();
        s.logout();
        assert!(s.cookies.is_empty());
        assert_eq!(s.device_id, device);
    }
}
