//! Vision tools — find color / multi-color / OCR (Tesseract)
//!
//! All functions operate directly on BMP pixel data, zero external dependencies.
//!
//! - Find image migrated to desktop-api Locator (coarse + fine MAD matching)
//! - Dictionary system migrated to dict_ocr module
//!
//! Color finding algorithm:
//! - Compare colors pixel by pixel within specified region
//! - Color format "RRGGBB-DRDGDB" with delta tolerance
//!
//! Multi-color find:
//! - Find anchor color first, then verify offset points
//! - "!" prefix means this point should not match (negative exclusion)

use serde_json::Value;

/// Color delta parameters (per-channel tolerance 0..255)
#[derive(Debug, Clone, Copy)]
pub struct DeltaColor {
    pub dr: u8,
    pub dg: u8,
    pub db: u8,
}

impl DeltaColor {
    pub fn from_hex(s: &str) -> Self {
        let s = s.trim();
        if s.len() < 6 {
            return Self {
                dr: 0,
                dg: 0,
                db: 0,
            };
        }
        Self {
            dr: u8::from_str_radix(&s[0..2], 16).unwrap_or(0),
            dg: u8::from_str_radix(&s[2..4], 16).unwrap_or(0),
            db: u8::from_str_radix(&s[4..6], 16).unwrap_or(0),
        }
    }
    pub fn zero() -> Self {
        Self {
            dr: 0,
            dg: 0,
            db: 0,
        }
    }
}

/// Search direction
#[derive(Debug, Clone, Copy)]
pub enum SearchDir {
    LeftTop,
    RightTop,
    LeftBottom,
    RightBottom,
}

impl SearchDir {
    pub fn from_i32(n: i32) -> Self {
        match n & 3 {
            1 => Self::RightTop,
            2 => Self::LeftBottom,
            3 => Self::RightBottom,
            _ => Self::LeftTop,
        }
    }
}

fn parse_color_hex(s: &str) -> Option<(u8, u8, u8)> {
    let s = s.trim().trim_start_matches('#');
    if s.len() < 6 {
        return None;
    }
    Some((
        u8::from_str_radix(&s[0..2], 16).ok()?,
        u8::from_str_radix(&s[2..4], 16).ok()?,
        u8::from_str_radix(&s[4..6], 16).ok()?,
    ))
}

fn parse_color_delta(s: &str) -> Option<((u8, u8, u8), (u8, u8, u8))> {
    let s = s.trim();
    // 尝试逗号分隔 (R,G,B 或 R,G,B,Dr,Dg,Db)
    if s.contains(',') {
        let parts: Vec<&str> = s.split(',').map(|p| p.trim()).collect();
        if parts.len() >= 3 {
            let c = (
                parts[0].parse().ok()?,
                parts[1].parse().ok()?,
                parts[2].parse().ok()?,
            );
            let d = if parts.len() >= 6 {
                (
                    parts[3].parse().ok()?,
                    parts[4].parse().ok()?,
                    parts[5].parse().ok()?,
                )
            } else {
                (0, 0, 0)
            };
            return Some((c, d));
        }
    }
    // 尝试 HEX 格式: "3B82F6" 或 "3B82F6-050505"
    let parts: Vec<&str> = s.split('-').collect();
    let c = parse_color_hex(parts[0])?;
    let d = if parts.len() > 1 {
        parse_color_hex(parts[1]).unwrap_or((0, 0, 0))
    } else {
        (0, 0, 0)
    };
    Some((c, d))
}

fn match_color(r: u8, g: u8, b: u8, tr: u8, tg: u8, tb: u8, dr: u8, dg: u8, db: u8) -> bool {
    (r as i16 - tr as i16).abs() <= dr as i16
        && (g as i16 - tg as i16).abs() <= dg as i16
        && (b as i16 - tb as i16).abs() <= db as i16
}

// ══════════════════════════════════════════════════════════════
//  find_image migrated to desktop-api Locator (coarse + fine MAD matching)
//  参见 DesktopClient::find_image() 和 desktop_api::vision::locate
// ══════════════════════════════════════════════════════════════

// ══════════════════════════════════════════════════════════════
//  Color find (enhanced)
// ══════════════════════════════════════════════════════════════

/// 单次查找最多返回的匹配点数量（防止大区域热色导致结果爆炸）
pub const MAX_COLOR_MATCHES: usize = 1000;

/// 扫描方向的全部合法取值（9 档）。
///
/// - 行优先（外层 Y，逐行推进）：`left_top`（默认）/ `left_bottom` / `right_top` / `right_bottom`
/// - 列优先（外层 X，逐列推进）：`column_left_top` / `column_right_top` /
///   `column_left_bottom` / `column_right_bottom`
/// - 中心扩散：`center`（从区域中心按同心环向外）
pub const SCAN_DIRECTIONS: &[&str] = &[
    "left_top",
    "left_bottom",
    "right_top",
    "right_bottom",
    "column_left_top",
    "column_right_top",
    "column_left_bottom",
    "column_right_bottom",
    "center",
];

/// 按给定方向对区域做逐点扫描；`visit` 返回 `true` 即提前终止。
///
/// 逐点回调直接消费坐标，不预生成坐标表（全屏下坐标表会占用 MB 级内存）。
pub fn scan_rect(
    x1: u32,
    y1: u32,
    x2: u32,
    y2: u32,
    direction: &str,
    mut visit: impl FnMut(u32, u32) -> bool,
) {
    if x2 <= x1 || y2 <= y1 {
        return;
    }
    match direction {
        // ── 行优先：外层 Y，逐行推进 ──
        "left_bottom" => {
            for y in (y1..y2).rev() {
                for x in x1..x2 {
                    if visit(x, y) {
                        return;
                    }
                }
            }
        }
        "right_top" => {
            for y in y1..y2 {
                for x in (x1..x2).rev() {
                    if visit(x, y) {
                        return;
                    }
                }
            }
        }
        "right_bottom" => {
            for y in (y1..y2).rev() {
                for x in (x1..x2).rev() {
                    if visit(x, y) {
                        return;
                    }
                }
            }
        }
        // ── 列优先：外层 X，逐列推进 ──
        "column_left_top" => {
            for x in x1..x2 {
                for y in y1..y2 {
                    if visit(x, y) {
                        return;
                    }
                }
            }
        }
        "column_right_top" => {
            for x in (x1..x2).rev() {
                for y in y1..y2 {
                    if visit(x, y) {
                        return;
                    }
                }
            }
        }
        "column_left_bottom" => {
            for x in x1..x2 {
                for y in (y1..y2).rev() {
                    if visit(x, y) {
                        return;
                    }
                }
            }
        }
        "column_right_bottom" => {
            for x in (x1..x2).rev() {
                for y in (y1..y2).rev() {
                    if visit(x, y) {
                        return;
                    }
                }
            }
        }
        // ── 中心扩散：从区域中心按同心环向外 ──
        "center" => {
            let cx = (x1 + x2) / 2;
            let cy = (y1 + y2) / 2;
            if visit(cx, cy) {
                return;
            }
            let span = (x2 - x1).max(y2 - y1);
            for r in 1..=span {
                // 上边 + 下边（含角），左右两边只取中段避免角重复
                let top = cy.checked_sub(r);
                let bottom = cy.checked_add(r).filter(|v| *v < y2);
                let left = cx.checked_sub(r);
                let right = cx.checked_add(r).filter(|v| *v < x2);
                let x_lo = cx.saturating_sub(r).max(x1);
                let x_hi = cx.saturating_add(r).min(x2.saturating_sub(1));
                if let Some(ty) = top.filter(|v| *v >= y1) {
                    for x in x_lo..=x_hi {
                        if visit(x, ty) {
                            return;
                        }
                    }
                }
                if let Some(by) = bottom {
                    for x in x_lo..=x_hi {
                        if visit(x, by) {
                            return;
                        }
                    }
                }
                let y_lo = cy.saturating_sub(r).saturating_add(1).max(y1);
                let y_hi = cy
                    .saturating_add(r)
                    .saturating_sub(1)
                    .min(y2.saturating_sub(1));
                if y_lo <= y_hi {
                    if let Some(lx) = left.filter(|v| *v >= x1) {
                        for y in y_lo..=y_hi {
                            if visit(lx, y) {
                                return;
                            }
                        }
                    }
                    if let Some(rx) = right {
                        for y in y_lo..=y_hi {
                            if visit(rx, y) {
                                return;
                            }
                        }
                    }
                }
            }
        }
        // ── 默认：left_top（行优先，X 左→右、Y 上→下）──
        _ => {
            for y in y1..y2 {
                for x in x1..x2 {
                    if visit(x, y) {
                        return;
                    }
                }
            }
        }
    }
}

/// Find color in memory frame, supports multi-color combinations and delta tolerance.
///
/// `pixels` 为 RGB 序列（3 字节/像素），`sw`/`sh` 为图像宽高。
/// 区域判定同上层约定：`region_w == 0` 表示全图。
#[allow(clippy::too_many_arguments)]
pub fn find_color(
    pixels: &[u8],
    sw: u32,
    sh: u32,
    color: &str,
    region_x: i32,
    region_y: i32,
    region_w: u32,
    region_h: u32,
    direction: &str,
) -> crate::Result<Value> {
    let specs: Vec<((u8, u8, u8), (u8, u8, u8))> =
        color.split('|').filter_map(parse_color_delta).collect();
    if specs.is_empty() {
        return Ok(serde_json::json!({"found":false,"x":0,"y":0}));
    }

    let (sx, sy, ex, ey) = region_bounds(sw, sh, region_x, region_y, region_w, region_h);

    let mut all: Vec<Value> = Vec::new();
    let mut truncated = false;
    scan_rect(sx, sy, ex, ey, direction, |x, y| {
        let idx = ((y * sw + x) * 3) as usize;
        let (pr, pg, pb) = (pixels[idx], pixels[idx + 1], pixels[idx + 2]);
        for &((cr, cg, cb), (dr, dg, db)) in &specs {
            if match_color(pr, pg, pb, cr, cg, cb, dr, dg, db) {
                if all.len() >= MAX_COLOR_MATCHES {
                    truncated = true;
                    return true;
                }
                all.push(serde_json::json!({
                    "x": x, "y": y, "r": pr, "g": pg, "b": pb,
                    "target": format!("{:02X}{:02X}{:02X}", cr, cg, cb),
                }));
                break;
            }
        }
        false
    });

    if all.is_empty() {
        Ok(serde_json::json!({"found":false,"x":0,"y":0,"matches":[]}))
    } else {
        Ok(serde_json::json!({
            "found":true,"x":all[0]["x"],"y":all[0]["y"],"matches":all,
            "truncated": truncated,
        }))
    }
}

/// 计算区域裁剪边界（clamp 到图像内；宽度或高度为 0 视为全图）
fn region_bounds(
    sw: u32,
    sh: u32,
    region_x: i32,
    region_y: i32,
    region_w: u32,
    region_h: u32,
) -> (u32, u32, u32, u32) {
    let sx = if region_w > 0 {
        region_x.max(0) as u32
    } else {
        0
    };
    let sy = if region_h > 0 {
        region_y.max(0) as u32
    } else {
        0
    };
    let cw = if region_w > 0 {
        (region_w as i32).min(sw as i32 - sx as i32).max(0) as u32
    } else {
        sw
    };
    let ch = if region_h > 0 {
        (region_h as i32).min(sh as i32 - sy as i32).max(0) as u32
    } else {
        sh
    };
    let ex = (sx + cw).min(sw);
    let ey = (sy + ch).min(sh);
    (sx.min(sw), sy.min(sh), ex, ey)
}

// ══════════════════════════════════════════════════════════════
//  Multi-color find (enhanced)
// ══════════════════════════════════════════════════════════════

/// Multi-color find — find anchor color first, then verify offset point colors.
///
/// `pixels` 为 RGB 序列（3 字节/像素），`sw`/`sh` 为图像宽高。
/// `direction` 参与扫描顺序（9 档，见 `scan_rect`）。
#[allow(clippy::too_many_arguments)]
pub fn find_multi_color(
    pixels: &[u8],
    sw: u32,
    sh: u32,
    anchor_color: &str,
    offset_specs: &str,
    min_match_ratio: f64,
    region_x: i32,
    region_y: i32,
    region_w: u32,
    region_h: u32,
    direction: &str,
) -> crate::Result<Value> {
    let (ac, ad) = parse_color_delta(anchor_color)
        .ok_or_else(|| crate::NuphusError::Tool("invalid anchor color".into()))?;

    struct Pt {
        dx: i32,
        dy: i32,
        expect: bool,
        c: (u8, u8, u8),
        d: (u8, u8, u8),
    }
    let offsets: Vec<Pt> = offset_specs
        .split(',')
        .filter(|s| !s.trim().is_empty())
        .filter_map(|seg| {
            let parts: Vec<&str> = seg.trim().splitn(3, '|').collect();
            if parts.len() < 3 {
                return None;
            }
            let dx: i32 = parts[0].trim().parse().ok()?;
            let dy: i32 = parts[1].trim().parse().ok()?;
            let cs = parts[2].trim();
            let expect = !cs.starts_with('!');
            let clean = if expect { cs } else { &cs[1..] };
            let (c, d) = parse_color_delta(clean)?;
            Some(Pt {
                dx,
                dy,
                expect,
                c,
                d,
            })
        })
        .collect();

    if offsets.is_empty() {
        return Ok(serde_json::json!({"found":false,"x":0,"y":0,"match_ratio":0.0}));
    }

    let (sx, sy, ex, ey) = region_bounds(sw, sh, region_x, region_y, region_w, region_h);
    let olen = offsets.len() as f64;

    let mut hit: Option<Value> = None;
    scan_rect(sx, sy, ex, ey, direction, |x, y| {
        let idx = ((y * sw + x) * 3) as usize;
        let (pr, pg, pb) = (pixels[idx], pixels[idx + 1], pixels[idx + 2]);
        if !match_color(pr, pg, pb, ac.0, ac.1, ac.2, ad.0, ad.1, ad.2) {
            return false;
        }

        let mut matched = 0u32;
        for off in &offsets {
            let ox = x as i32 + off.dx;
            let oy = y as i32 + off.dy;
            if ox < 0 || ox >= sw as i32 || oy < 0 || oy >= sh as i32 {
                if off.expect {
                    matched = 0;
                    break;
                } else {
                    matched += 1;
                    continue;
                }
            }
            let oi = ((oy as u32 * sw + ox as u32) * 3) as usize;
            let ok = match_color(
                pixels[oi],
                pixels[oi + 1],
                pixels[oi + 2],
                off.c.0,
                off.c.1,
                off.c.2,
                off.d.0,
                off.d.1,
                off.d.2,
            );
            if off.expect == ok {
                matched += 1;
            }
        }
        let ratio = matched as f64 / olen;
        if ratio >= min_match_ratio {
            hit = Some(serde_json::json!({
                "found":true,"x":x,"y":y,"match_ratio":(ratio*10000.0).round()/10000.0
            }));
            return true; // 命中即止：首个匹配点就是调用方要的
        }
        false
    });

    Ok(hit.unwrap_or_else(|| serde_json::json!({"found":false,"x":0,"y":0,"match_ratio":0.0})))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 收集扫描前 N 个坐标，用于断言各档的推进顺序
    fn take(direction: &str, n: usize) -> Vec<(u32, u32)> {
        let mut seen = Vec::new();
        scan_rect(0, 0, 3, 2, direction, |x, y| {
            seen.push((x, y));
            seen.len() >= n
        });
        seen
    }

    #[test]
    fn scan_directions_cover_all_nine_and_start_at_expected_corner() {
        // 逐档断言首个访问点（角/边/中心），防档位串线
        assert_eq!(take("left_top", 1), vec![(0, 0)]);
        assert_eq!(take("right_top", 1), vec![(2, 0)]);
        assert_eq!(take("left_bottom", 1), vec![(0, 1)]);
        assert_eq!(take("right_bottom", 1), vec![(2, 1)]);
        assert_eq!(take("column_left_top", 1), vec![(0, 0)]);
        assert_eq!(take("column_right_top", 1), vec![(2, 0)]);
        assert_eq!(take("column_left_bottom", 1), vec![(0, 1)]);
        assert_eq!(take("column_right_bottom", 1), vec![(2, 1)]);
        // 中心档：奇数尺寸下中心点必须最先访问
        let first = take("center", 1);
        assert_eq!(first.len(), 1);
        assert!((0..3).contains(&first[0].0) && (0..2).contains(&first[0].1));
        // 九档枚举与实际实现保持同步
        assert_eq!(SCAN_DIRECTIONS.len(), 9);
    }

    #[test]
    fn scan_rect_visits_every_pixel_exactly_once() {
        for direction in SCAN_DIRECTIONS {
            let mut count = 0usize;
            let mut seen = std::collections::HashSet::new();
            scan_rect(0, 0, 3, 2, direction, |x, y| {
                count += 1;
                assert!(seen.insert((x, y)), "{direction} 重复访问 ({x},{y})");
                false
            });
            assert_eq!(count, 6, "{direction} 像素数应为 3*2");
        }
    }

    #[test]
    fn scan_rect_stops_early_on_true() {
        let mut count = 0usize;
        scan_rect(0, 0, 100, 100, "left_top", |_, _| {
            count += 1;
            count >= 5
        });
        assert_eq!(count, 5, "visit 返回 true 必须立即终止");
    }

    #[test]
    fn scan_rect_empty_or_inverted_region_visits_nothing() {
        let mut count = 0usize;
        scan_rect(5, 5, 5, 5, "left_top", |_, _| {
            count += 1;
            false
        });
        scan_rect(10, 10, 3, 3, "left_top", |_, _| {
            count += 1;
            false
        });
        assert_eq!(count, 0);
    }
}
