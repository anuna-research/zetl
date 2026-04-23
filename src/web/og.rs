//! Open Graph image generation for the static site builder.
//!
//! For each built page we render a 1200×630 PNG containing the page title
//! and vault name. A user-supplied background PNG (dropped at
//! `.ztl/static/og-background.png` in the vault, or at the equivalent path
//! inside a custom theme) is composited underneath when present; otherwise
//! a flat dark background is used.
//!
//! See `themes/default/static/og-PROMPT.md` for the Midjourney prompt that
//! produces a suitable background template.

use ab_glyph::{Font, FontRef, PxScale, ScaleFont};
use anyhow::{Context, Result};
use image::{codecs::png::PngEncoder, ExtendedColorType, ImageEncoder, Rgba, RgbaImage};
use imageproc::drawing::draw_text_mut;
use std::path::Path;

pub const OG_WIDTH: u32 = 1200;
pub const OG_HEIGHT: u32 = 630;

/// Inter Bold, SIL OFL — bundled under themes/default/static/Inter-Bold.ttf.
static FONT_BOLD_BYTES: &[u8] = include_bytes!("../../themes/default/static/Inter-Bold.ttf");
static FONT_REGULAR_BYTES: &[u8] = include_bytes!("../../themes/default/static/Inter-Regular.ttf");

/// Render an OG card PNG and return its bytes.
///
/// `title` is the primary page title (large). `subtitle` is the vault name
/// (smaller, above the title). `background` is an optional pre-loaded
/// RGBA image to composite underneath the text.
pub fn render_og_png(
    title: &str,
    subtitle: &str,
    background: Option<&RgbaImage>,
) -> Result<Vec<u8>> {
    let font_bold =
        FontRef::try_from_slice(FONT_BOLD_BYTES).context("loading bundled Inter-Bold font")?;
    let font_regular = FontRef::try_from_slice(FONT_REGULAR_BYTES)
        .context("loading bundled Inter-Regular font")?;

    // Start from background (resized/cropped to fit) or a flat dark canvas.
    let mut img: RgbaImage = match background {
        Some(bg) => fit_cover(bg, OG_WIDTH, OG_HEIGHT),
        None => RgbaImage::from_pixel(OG_WIDTH, OG_HEIGHT, Rgba([13, 17, 23, 255])),
    };

    // Subtle dark gradient overlay at the bottom for text legibility.
    apply_bottom_scrim(&mut img);

    // Layout:
    //   subtitle (vault name) at 48px, positioned at y=360
    //   title at 88px bold, wrapped, starting at y=410
    //   a small "ztl" wordmark at bottom-right
    let margin_x: i32 = 72;
    let subtitle_scale = PxScale::from(42.0);
    let title_scale = PxScale::from(88.0);
    let wordmark_scale = PxScale::from(28.0);

    let subtitle_color = Rgba([180, 200, 220, 255]);
    let title_color = Rgba([245, 245, 245, 255]);
    let wordmark_color = Rgba([140, 160, 180, 255]);

    // Subtitle (single line, truncated with ellipsis if too wide).
    let subtitle_line = truncate_to_fit(
        subtitle,
        &font_regular,
        subtitle_scale,
        OG_WIDTH as i32 - 2 * margin_x,
    );
    draw_text_mut(
        &mut img,
        subtitle_color,
        margin_x,
        350,
        subtitle_scale,
        &font_regular,
        &subtitle_line,
    );

    // Title (wrapped across up to 3 lines).
    let max_title_width = OG_WIDTH as i32 - 2 * margin_x;
    let title_lines = wrap_text(title, &font_bold, title_scale, max_title_width, 3);
    let line_height = 104i32;
    for (i, line) in title_lines.iter().enumerate() {
        draw_text_mut(
            &mut img,
            title_color,
            margin_x,
            410 + (i as i32) * line_height,
            title_scale,
            &font_bold,
            line,
        );
    }

    // Wordmark bottom-right.
    let wordmark = "ztl";
    let wordmark_width = text_width(wordmark, &font_bold, wordmark_scale);
    draw_text_mut(
        &mut img,
        wordmark_color,
        OG_WIDTH as i32 - margin_x - wordmark_width,
        OG_HEIGHT as i32 - 60,
        wordmark_scale,
        &font_bold,
        wordmark,
    );

    let mut buf: Vec<u8> = Vec::with_capacity(64 * 1024);
    let encoder = PngEncoder::new(&mut buf);
    encoder
        .write_image(&img, OG_WIDTH, OG_HEIGHT, ExtendedColorType::Rgba8)
        .context("encoding OG png")?;
    Ok(buf)
}

/// Load the user-supplied background from disk if present. Searches, in
/// order: `<theme_static>/og-background.png`, `<vault>/.ztl/static/og-background.png`.
pub fn load_background(vault_root: &Path, theme_static_dir: Option<&Path>) -> Option<RgbaImage> {
    let candidates = [
        theme_static_dir.map(|d| d.join("og-background.png")),
        Some(vault_root.join(".ztl/static/og-background.png")),
    ];
    for path in candidates.into_iter().flatten() {
        if path.is_file() {
            if let Ok(img) = image::open(&path) {
                return Some(img.to_rgba8());
            }
        }
    }
    None
}

fn text_width(s: &str, font: &FontRef<'_>, scale: PxScale) -> i32 {
    let scaled = font.as_scaled(scale);
    let mut w = 0.0f32;
    let mut prev = None::<char>;
    for c in s.chars() {
        let g = scaled.scaled_glyph(c);
        if let Some(p) = prev {
            w += scaled.kern(font.glyph_id(p), g.id);
        }
        w += scaled.h_advance(g.id);
        prev = Some(c);
    }
    w as i32
}

fn truncate_to_fit(s: &str, font: &FontRef<'_>, scale: PxScale, max_width: i32) -> String {
    if text_width(s, font, scale) <= max_width {
        return s.to_string();
    }
    let ellipsis = "…";
    let mut chars: Vec<char> = s.chars().collect();
    while !chars.is_empty() {
        chars.pop();
        let candidate: String = chars.iter().collect::<String>() + ellipsis;
        if text_width(&candidate, font, scale) <= max_width {
            return candidate;
        }
    }
    ellipsis.to_string()
}

fn wrap_text(
    s: &str,
    font: &FontRef<'_>,
    scale: PxScale,
    max_width: i32,
    max_lines: usize,
) -> Vec<String> {
    let words: Vec<&str> = s.split_whitespace().collect();
    if words.is_empty() {
        return vec![String::new()];
    }
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    for w in words {
        let candidate = if current.is_empty() {
            w.to_string()
        } else {
            format!("{current} {w}")
        };
        if text_width(&candidate, font, scale) <= max_width {
            current = candidate;
        } else {
            if !current.is_empty() {
                lines.push(current);
            }
            // Word itself may exceed max_width; break it with truncation if so.
            current = if text_width(w, font, scale) > max_width {
                truncate_to_fit(w, font, scale, max_width)
            } else {
                w.to_string()
            };
            if lines.len() + 1 >= max_lines {
                // Remaining content goes into the last line, truncated.
                break;
            }
        }
    }
    if !current.is_empty() && lines.len() < max_lines {
        lines.push(current);
    }
    // If text didn't fit in max_lines, truncate the last line.
    if lines.len() == max_lines {
        if let Some(last) = lines.last_mut() {
            *last = truncate_to_fit(last, font, scale, max_width);
        }
    }
    lines
}

/// Crop-fit `src` into an (w×h) canvas (centre-cropped to preserve aspect).
fn fit_cover(src: &RgbaImage, w: u32, h: u32) -> RgbaImage {
    let (sw, sh) = src.dimensions();
    let src_ratio = sw as f32 / sh as f32;
    let dst_ratio = w as f32 / h as f32;
    let (crop_w, crop_h) = if src_ratio > dst_ratio {
        // Source is wider than destination — crop sides.
        ((sh as f32 * dst_ratio) as u32, sh)
    } else {
        // Source is taller — crop top/bottom.
        (sw, (sw as f32 / dst_ratio) as u32)
    };
    let x = (sw - crop_w) / 2;
    let y = (sh - crop_h) / 2;
    let cropped = image::imageops::crop_imm(src, x, y, crop_w, crop_h).to_image();
    image::imageops::resize(&cropped, w, h, image::imageops::FilterType::Lanczos3)
}

/// Darken the bottom ~55% of the image with a linear gradient so the text
/// stays legible across varied backgrounds.
fn apply_bottom_scrim(img: &mut RgbaImage) {
    let h = img.height();
    let scrim_start = (h as f32 * 0.45) as u32;
    for y in scrim_start..h {
        let t = (y - scrim_start) as f32 / (h - scrim_start) as f32;
        let darken = (t.powf(1.4) * 180.0) as u8;
        for x in 0..img.width() {
            let p = img.get_pixel_mut(x, y);
            p.0[0] = p.0[0].saturating_sub(darken);
            p.0[1] = p.0[1].saturating_sub(darken);
            p.0[2] = p.0[2].saturating_sub(darken);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_without_background() {
        let bytes = render_og_png("Hello World", "My Vault", None).unwrap();
        assert!(bytes.len() > 4096, "og png should be non-trivial");
        // PNG magic.
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
    }

    #[test]
    fn renders_very_long_title() {
        let long = "A very long page title that should wrap onto multiple lines and then be truncated with an ellipsis when it runs past the third line of available space";
        let bytes = render_og_png(long, "Vault", None).unwrap();
        assert!(bytes.len() > 4096);
    }

    #[test]
    fn renders_with_background() {
        let bg = RgbaImage::from_pixel(800, 420, Rgba([40, 80, 120, 255]));
        let bytes = render_og_png("Title", "Vault", Some(&bg)).unwrap();
        assert!(bytes.len() > 4096);
    }

    #[test]
    fn wrap_respects_max_lines() {
        let font = FontRef::try_from_slice(FONT_BOLD_BYTES).unwrap();
        let lines = wrap_text(
            "one two three four five six seven eight nine ten",
            &font,
            PxScale::from(88.0),
            400,
            3,
        );
        assert!(lines.len() <= 3);
    }
}
