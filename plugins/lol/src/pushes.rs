//! 英雄联盟在选英雄和对局里的两条推送（ADR 043 D7：原来的两个覆盖层改成推送，本体的覆盖层整个去掉了）：
//!
//! - **选英雄**（[`champ_select`]，清单 `[[pushes]]` 的 `champ-select`，页面 `ui://kotone/lol/champ-select`）：
//!   选英雄时一张卡片——海斗是我方 5 人的评级和胜率、备选席里比你现在高的；峡谷是每个人这个位置的强度。
//!   同一次选英雄一个 key，重随、换英雄时原地更新；选英雄结束时再推一次（标上“结束了”），带一句给她的说明。
//! - **海克斯三选一**（[`augment_pick`]，`augment-pick`）：海斗对局里截图识别三张卡片，一行三格（左 / 中 / 右：
//!   首选 / 可选 / 可刷新）+ 她说一句“中间那个”；卡片在屏幕上时每秒看一次，刷新了哪张就原地换哪一格，最好的那张变了
//!   才再说；卡片没了就收起（`push_done`）。截图识别固定用本地模型（宿主管，和全局的选择无关）；一个都没装时推一张卡片问
//!   要不要下载（页面 `ui://kotone/lol/augment-ocr`），用不了时一行说清楚原因（每局最多一次）。
//!
//! 评级和三选一的判断规则在 [`rules`]（照 AramHex，MIT）。开局战绩、这局总结两条卡片在 `watcher/auto_cards.rs`。
//! 插件只说游戏里的东西是什么（固定规则算出来的判断、数据源的数字，ADR 043 D2）；不读游戏内存、不注入、不抓包。

pub(crate) mod augment_pick;
pub(crate) mod champ_select;
pub(crate) mod rules;

use std::sync::Arc;

use kotone_plugin_api::Host;

use crate::client::live::LiveGame;
use crate::client::Lcu;
use crate::LolPlugin;

/// 选英雄卡片的页面（原来是选英雄的覆盖层，地址沿用：外部宿主可能已经认它，ADR 043 D7）。
pub const PAGE_CHAMP_SELECT: &str = "ui://kotone/lol/champ-select";
/// 问要不要下载本地文字识别模型的那张卡片的页面（海克斯三选一要用）。
pub const PAGE_AUGMENT_OCR: &str = "ui://kotone/lol/augment-ocr";
/// 清单 `[[pushes]]` 里这两条推送的 id。
pub const PUSH_CHAMP_SELECT: &str = "champ-select";
pub const PUSH_AUGMENT_PICK: &str = "augment-pick";
/// 海克斯大乱斗（海斗）的队列。
pub(crate) const MAYHEM_QUEUE: i64 = 2400;
/// 数据从哪来：卡片上的来源标签（ADR 043 D8，页面的 `SourceTag`，id 和 `stats_tools.rs` 的来源一样）。
pub(crate) const SOURCE_QQ101: &str = "qq101";

/// 现在的 Unix 毫秒。
pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// 两条推送现在的样子（防止重复推同样的东西、识别叠在一起）。
#[derive(Default)]
pub(crate) struct PushState {
    pub(crate) champ_select: champ_select::CardState,
    pub(crate) pick: augment_pick::PickState,
}

impl LolPlugin {
    /// 后台每一轮的最后（客户端开着时）：选英雄时推 / 更新选英雄卡片；告诉三选一的循环现在是不是在海斗对局里。
    pub(crate) async fn update_pushes(
        &self,
        host: &Arc<dyn Host>,
        lcu: &Lcu,
        phase: &str,
        live: Option<&LiveGame>,
    ) {
        self.champ_select_card(host.as_ref(), lcu, phase).await;
        self.note_live_game(host.as_ref(), phase, live);
    }

    /// 客户端关了、插件或“读客户端数据”关了：选英雄卡片不再更新，三选一的那一行收起。
    pub(crate) fn reset_pushes(&self, host: &dyn Host) {
        self.pushes.lock().unwrap().champ_select = champ_select::CardState::default();
        self.note_live_game(host, "", None);
    }
}
