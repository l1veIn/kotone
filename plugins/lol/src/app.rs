//! 英雄联盟的 MCP App（ADR 039 D5、D6；MCP 方案第 5 期）：页面是 `app/` 里独立的 HTML（Vite + Svelte，
//! `vite-plugin-singlefile` 打成一个文件，`build.rs` 编进来），不再编进宿主的 Svelte。Kotone（回环代理里的沙盒
//! iframe）和 Claude 这类外部宿主（`kotone.exe mcp` 的 `resources/read`）显示的是同一份。
//!
//! - 几个页面共用一个打包：[`resource`] 在 `<head>` 里插一行 `<meta name="kotone:page" content="<页面>">`，
//!   App 启动时按它选页面。
//! - 做成 App 的页面列在 [`APP_PAGES`]：这局、战绩、单局详情、英雄、强度榜，每一页都能单独打开（聊天卡片、
//!   推送的卡片、`ui.open`、外部宿主的 `resources/read`），打开以后在 App 里自己导航；还有几张只在推送里出现的卡片
//!   （ADR 043 D7）：选英雄（原来的选英雄覆盖层）、问要不要下载本地文字识别模型、进房间时的今天的状态（B 轮）、海斗三选一、你阵亡时对面的装备（B 轮）。设置页是宿主的 Svelte 页面，不是 App。宿主的 `ui/index.ts`（`app: true`）和浏览器预览的模拟（`pluginsMock.ts`）要和这里一致，
//!   桌面端的测试 `plugin_apps::tests::app_pages_agree_with_the_frontend` 管着。
//! - 每一页要的数据开关照清单 `pages` 的 `needs`（这局、战绩、单局详情要“读客户端数据”，英雄、强度榜要
//!   “用网上的统计”），宿主读资源前查（`plugin_apps::read`）。
//! - 图片不进安装包（ADR 039 D6）：页面在 Kotone 里先从回环代理的 `/assets/lol/<路径>` 取（路径照客户端的
//!   资源接口写，例如 `lol-game-data/assets/v1/champion-icons/<英雄 id>.png`，由 [`crate::LolPlugin`] 的 `asset`
//!   给），取不到、或者在别的宿主里，用 CommunityDragon（[`CDRAGON`]，写进资源的 CSP 声明）。五页的图都是这两处来的，
//!   CSP 一样。

use kotone_plugin_api::{Csp, Resource};

use crate::pages::{PAGE_GAME, PAGE_HOME, PAGE_MATCH, PAGE_PLAYER, PAGE_TODAY};
use crate::pushes::{PAGE_AUGMENT_OCR, PAGE_AUGMENT_PICK, PAGE_CHAMP_SELECT, PAGE_ENEMY_BUILD};
use crate::stats_tools::{PAGE_CHAMPION, PAGE_TIER};

include!(concat!(env!("OUT_DIR"), "/app_bundle.rs"));

/// App 还没打包时的占位页（`cargo test`、CI 的 Rust 检查不依赖前端构建）。打好了时用不到。
#[allow(dead_code)]
const PLACEHOLDER: &str = "<!doctype html>\n<html lang=\"zh-CN\"><head><meta charset=\"utf-8\"><meta name=\"color-scheme\" content=\"light dark\"><title>英雄联盟</title></head>\n<body style=\"font: 14px system-ui, sans-serif; padding: 16px\"><p>英雄联盟 App 还没构建：在仓库里跑 <code>pnpm -C apps/desktop build:apps</code>，再重新编译 Kotone。</p><p>The League of Legends app has not been built yet: run <code>pnpm -C apps/desktop build:apps</code> and rebuild Kotone.</p></body></html>\n";

/// 做成 MCP App 的页面（清单 `pages` 里的地址，照清单的顺序）。
pub const APP_PAGES: &[&str] = &[
    PAGE_HOME,
    PAGE_MATCH,
    PAGE_PLAYER,
    PAGE_GAME,
    PAGE_CHAMPION,
    PAGE_TIER,
    PAGE_CHAMP_SELECT,
    PAGE_AUGMENT_OCR,
    PAGE_TODAY,
    PAGE_AUGMENT_PICK,
    PAGE_ENEMY_BUILD,
];

/// CommunityDragon：客户端资源的公开镜像（英雄头像、图标），客户端没开或在别的宿主里时用。
pub const CDRAGON: &str = "https://raw.communitydragon.org";

/// 这个页面的 MCP App 资源；不是 App 的页面给 `None`。
pub fn resource(uri: &str) -> Option<Resource> {
    if !APP_PAGES.contains(&uri) {
        return None;
    }
    let page = uri.rsplit('/').next()?;
    Some(
        Resource::app(uri, with_page(BUNDLE, page))
            .csp(Csp {
                resource_domains: vec![CDRAGON.to_string()],
                ..Csp::default()
            })
            // 页面自己画底色和边（在 Kotone 的插件页里是整块内容区）
            .prefers_border(false),
    )
}

/// 在 `<head>` 后面插一行 `<meta name="kotone:page" content="<page>">`（没有 `<head>` 就插在最前面）。
fn with_page(html: &str, page: &str) -> String {
    let meta = format!("<meta name=\"kotone:page\" content=\"{page}\">");
    // `<head>` 或 `<head …>`（不能是 `<header>`）
    let at = ["<head>", "<head "]
        .iter()
        .filter_map(|tag| html.find(tag))
        .min()
        .and_then(|start| html[start..].find('>').map(|end| start + end + 1));
    match at {
        Some(at) => format!("{}{meta}{}", &html[..at], &html[at..]),
        None => format!("{meta}{html}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 插件开启时始终可以打开主页；各数据页按自己的数据源开关提供。
    #[test]
    fn every_page_but_settings_is_an_app() {
        let m = crate::manifest();
        let pages: Vec<&str> = m
            .pages
            .iter()
            .map(|p| p.uri.as_str())
            .filter(|uri| *uri != crate::pages::PAGE_SETTINGS)
            .collect();
        assert_eq!(pages, APP_PAGES, "清单的页面和 APP_PAGES 对不上");
        for uri in APP_PAGES {
            let page = m.page(uri).unwrap();
            assert_eq!(page.needs.is_empty(), *uri == PAGE_HOME, "{uri}");
        }
    }

    #[test]
    fn the_pages_are_apps_and_pass_the_manifest_check() {
        for (uri, page) in [
            (PAGE_HOME, "home"),
            (PAGE_MATCH, "match"),
            (PAGE_PLAYER, "player"),
            (PAGE_GAME, "game"),
            (PAGE_CHAMPION, "champion"),
            (PAGE_TIER, "tier"),
            (PAGE_CHAMP_SELECT, "champ-select"),
            (PAGE_AUGMENT_OCR, "augment-ocr"),
            (PAGE_TODAY, "today"),
            (PAGE_AUGMENT_PICK, "augment-pick"),
            (PAGE_ENEMY_BUILD, "enemy-build"),
        ] {
            let r = resource(uri).expect("是 App");
            crate::manifest().check_resource(&r).unwrap();
            // 图：Kotone 里走回环代理（宿主自己加），别的宿主里直接用 CommunityDragon
            assert_eq!(r.ui.csp.resource_domains, [CDRAGON]);
            assert_eq!(r.ui.prefers_border, Some(false));
            let marker = format!("<meta name=\"kotone:page\" content=\"{page}\">");
            assert!(
                r.text.contains(&marker),
                "{}",
                &r.text[..r.text.len().min(300)]
            );
            if !BUILT {
                assert!(r.text.contains("build:apps"), "占位页说清楚怎么打包");
            }
        }
        // 设置是宿主里的 Svelte 页面；海克斯评分原来只是覆盖层，覆盖层去掉了（ADR 043 D7），它从来不是页面
        for uri in [
            crate::pages::PAGE_SETTINGS,
            "ui://kotone/lol/augment-scores",
            "ui://kotone/lol/nope",
        ] {
            assert!(resource(uri).is_none(), "{uri}");
        }
    }

    #[test]
    fn the_page_marker_goes_into_the_head() {
        assert_eq!(
            with_page("<!doctype html><html><head><title>x</title></head></html>", "tier"),
            "<!doctype html><html><head><meta name=\"kotone:page\" content=\"tier\"><title>x</title></head></html>"
        );
        assert_eq!(
            with_page("<html><head lang=\"zh\">", "a"),
            "<html><head lang=\"zh\"><meta name=\"kotone:page\" content=\"a\">"
        );
        assert_eq!(
            with_page("<p>x</p>", "a"),
            "<meta name=\"kotone:page\" content=\"a\"><p>x</p>"
        );
    }
}
