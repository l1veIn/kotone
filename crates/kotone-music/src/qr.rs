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

#[cfg(test)]
mod tests {
    #[test]
    fn renders_an_svg() {
        let svg = super::svg("https://music.163.com/login?codekey=abc").unwrap();
        assert!(svg.starts_with("<?xml") || svg.starts_with("<svg"));
        assert!(svg.contains("<svg") && svg.contains("</svg>"));
    }
}
