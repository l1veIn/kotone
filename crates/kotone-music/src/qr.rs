//! 扫码登录的二维码：画成 SVG 字符串，界面直接显示（不用前端再装二维码库）。

pub(crate) fn svg(content: &str) -> Result<String, String> {
    let code = qrcode::QrCode::new(content.as_bytes()).map_err(|e| e.to_string())?;
    Ok(code
        .render::<qrcode::render::svg::Color>()
        .min_dimensions(220, 220)
        .quiet_zone(true)
        .dark_color(qrcode::render::svg::Color("#000000"))
        .light_color(qrcode::render::svg::Color("#ffffff"))
        .build())
}

/// 终端里能扫的二维码（每个字符上下两格，`kotone-cli music login` 用）。
pub fn text(content: &str) -> Result<String, String> {
    let code = qrcode::QrCode::new(content.as_bytes()).map_err(|e| e.to_string())?;
    Ok(code
        .render::<qrcode::render::unicode::Dense1x2>()
        .dark_color(qrcode::render::unicode::Dense1x2::Light)
        .light_color(qrcode::render::unicode::Dense1x2::Dark)
        .quiet_zone(true)
        .build())
}

#[cfg(test)]
mod tests {
    #[test]
    fn renders_an_svg() {
        let svg = super::svg("https://music.163.com/login?codekey=abc").unwrap();
        assert!(svg.starts_with("<?xml") || svg.starts_with("<svg"));
        assert!(svg.contains("<svg") && svg.contains("</svg>"));
        let text = super::text("https://music.163.com/login?codekey=abc").unwrap();
        assert!(text.lines().count() > 10);
    }
}
