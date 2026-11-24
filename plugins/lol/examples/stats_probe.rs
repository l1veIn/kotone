//! 真接口跑一遍“用网上的统计”（ADR 038 第 3 步）：腾讯 101、aramhex-data、外号和名字表。每个接口请求一次，
//! 缓存放在 `target/lol-stats-probe/`。
//!
//! ```text
//! cargo run -p kotone-plugin-lol --example stats_probe
//! ```

use kotone_plugin_lol::stats::{aramhex, Stats};

#[tokio::main]
async fn main() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/lol-stats-probe");
    let stats = Stats::new(dir);
    let started = std::time::Instant::now();
    let patch = stats.patch().await;
    println!("版本：{patch:?}（{:?}）", started.elapsed());

    let names = stats.nicknames().await.expect("外号表");
    let irelia = names.find("刀妹").expect("刀妹");
    println!(
        "刀妹 → {irelia} {}（英文 id {:?}）",
        names.label(irelia),
        names.aliases.get(&irelia)
    );

    let statics = stats.statics().await;
    println!(
        "名字表：装备 {} 个、符文 {} 个、召唤师技能 {} 个；3031 = {}",
        statics.items.len(),
        statics.runes.len(),
        statics.spells.len(),
        statics.item(3031)
    );

    let t = std::time::Instant::now();
    let tier = stats.tier_list("TOP").await.expect("上路强度榜");
    let (date, rows) = &tier.value;
    println!(
        "上路强度榜 {date}：{} 个英雄，第一 {}（{}，胜率 {:?}）（{:?}）",
        rows.len(),
        names.short(rows[0].champion_id),
        rows[0].tier,
        rows[0].win_rate,
        t.elapsed()
    );

    let t = std::time::Instant::now();
    let c = stats.champion(irelia, "TOP").await.expect("艾瑞莉娅上路");
    println!(
        "艾瑞莉娅上路：难打 {:?}，好打 {:?}，核心 {:?}，符文 {:?}，加点 {:?}/{:?}，技能 {:?}（{:?}）",
        c.bad_against.iter().take(3).map(|m| names.short(m.champion_id)).collect::<Vec<_>>(),
        c.good_against.iter().take(3).map(|m| names.short(m.champion_id)).collect::<Vec<_>>(),
        c.build.core.first().map(|o| o.items.iter().map(|i| statics.item(*i)).collect::<Vec<_>>()),
        c.runes.first().map(|r| r.primary.iter().map(|x| statics.rune(*x)).collect::<Vec<_>>()),
        c.skill_priority,
        c.skill_order.iter().take(6).collect::<Vec<_>>(),
        c.spells.first().map(|p| p.spells.map(|x| statics.spell(x))),
        t.elapsed()
    );

    let days: Vec<String> = (0..7)
        .map(|d| {
            // 粗略：按 UTC+8 往前数（和插件里一样的算法不对外暴露，这里只是探测）
            let secs = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs()
                + 8 * 3600
                - d * 86_400;
            let days = (secs / 86_400) as i64;
            let z = days + 719_468;
            let era = z.div_euclid(146_097);
            let doe = z - era * 146_097;
            let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
            let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
            let mp = (5 * doy + 2) / 153;
            let dd = doy - (153 * mp + 2) / 5 + 1;
            let m = if mp < 10 { mp + 3 } else { mp - 9 };
            let y = yoe + era * 400 + i64::from(m <= 2);
            format!("{y:04}{m:02}{dd:02}")
        })
        .collect();
    let mayhem = stats.mayhem_champions(&days).await.expect("海斗英雄榜");
    println!(
        "海斗英雄榜 {}：{} 个，第一 {}（胜率 {:?}）",
        mayhem.value.0,
        mayhem.value.1.len(),
        names.short(mayhem.value.1[0].champion_id),
        mayhem.value.1[0].win_rate
    );
    let augs = stats.mayhem_augments().await.expect("海斗海克斯榜");
    println!("海斗海克斯榜：{} 个", augs.value.len());

    let t = std::time::Instant::now();
    let hex = stats.aramhex().await.expect("aramhex-data");
    println!("{}（{:?}）", aramhex::attribution(&hex), t.elapsed());
    for (id, s) in hex.best_for(irelia, 5, 300) {
        println!(
            "  艾瑞莉娅 · {}：{} 局，胜率 {:.1}%，比平均 {:+.1}",
            hex.augments
                .get(&id)
                .map(|a| a.name.as_str())
                .unwrap_or("?"),
            s.games,
            s.win_rate * 100.0,
            s.delta
        );
    }
    // 第二次：都从缓存来
    let t = std::time::Instant::now();
    let _ = stats.champion(irelia, "TOP").await;
    let _ = stats.aramhex().await;
    println!("第二次（缓存）：{:?}", t.elapsed());
}
