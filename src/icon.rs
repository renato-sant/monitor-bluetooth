//! Desenha o ícone da bandeja: anel de progresso + porcentagem, com antialiasing.

use windows::core::w;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::WindowsAndMessaging::*;

const GREEN: [f32; 3] = [61.0, 214.0, 102.0];
const YELLOW: [f32; 3] = [250.0, 192.0, 20.0];
const RED: [f32; 3] = [235.0, 70.0, 70.0];

/// `level = None` desenha só a trilha (ícone do app, sem dispositivos).
pub fn make(size: i32, level: Option<u8>, light_theme: bool) -> HICON {
    let bgra = render(size, level, light_theme);
    unsafe { create_icon(size, &bgra) }
}

/// Pixels BGRA (alfa não pré-multiplicado), top-down.
pub fn render(size: i32, level: Option<u8>, light_theme: bool) -> Vec<u32> {
    let n = size as usize;
    let fg = match level {
        Some(l) if l >= 50 => GREEN,
        Some(l) if l >= 20 => YELLOW,
        _ => RED,
    };
    let track: [f32; 3] = if light_theme { [175.0; 3] } else { [95.0; 3] };
    let text_rgb: [f32; 3] = if light_theme { [20.0; 3] } else { [255.0; 3] };

    // 1) Anel com supersampling 4x4.
    let s = size as f32;
    let c = s / 2.0;
    let r_out = c - 0.25;
    let thick = (s * 0.11).max(1.75);
    let r_in = r_out - thick;
    let frac_fill = level.map(|l| l as f32 / 100.0).unwrap_or(0.0);
    const SS: usize = 4;

    let mut rgba = vec![[0f32; 4]; n * n];
    for y in 0..n {
        for x in 0..n {
            let (mut a_fg, mut a_tr) = (0f32, 0f32);
            for sy in 0..SS {
                for sx in 0..SS {
                    let dx = x as f32 + (sx as f32 + 0.5) / SS as f32 - c;
                    let dy = y as f32 + (sy as f32 + 0.5) / SS as f32 - c;
                    let d = (dx * dx + dy * dy).sqrt();
                    if d < r_in || d > r_out {
                        continue;
                    }
                    let mut ang = dx.atan2(-dy); // 0 = 12h, sentido horário
                    if ang < 0.0 {
                        ang += std::f32::consts::TAU;
                    }
                    if ang / std::f32::consts::TAU <= frac_fill {
                        a_fg += 1.0;
                    } else {
                        a_tr += 1.0;
                    }
                }
            }
            let total = (SS * SS) as f32;
            let (a_fg, a_tr) = (a_fg / total, a_tr / total);
            let a = a_fg + a_tr;
            if a > 0.0 {
                let mut px = [0f32; 4];
                for k in 0..3 {
                    px[k] = (fg[k] * a_fg + track[k] * a_tr) / a;
                }
                px[3] = a;
                rgba[y * n + x] = px;
            }
        }
    }

    // 2) Texto (100% não mostra número: o anel cheio já diz tudo).
    let text = match level {
        Some(l) if l < 100 => Some(l.to_string()),
        Some(_) => None,
        None => Some("–".to_string()),
    };
    if let Some(text) = text {
        let cov = render_text(size, &text);
        for (px, &t) in rgba.iter_mut().zip(cov.iter()) {
            if t <= 0.0 {
                continue;
            }
            let a = t + px[3] * (1.0 - t);
            for k in 0..3 {
                px[k] = (text_rgb[k] * t + px[k] * px[3] * (1.0 - t)) / a;
            }
            px[3] = a;
        }
    }

    // 3) Monta o HICON (32bpp com alfa).
    rgba.iter()
        .map(|p| {
            let a = (p[3].clamp(0.0, 1.0) * 255.0).round() as u32;
            if a == 0 {
                return 0;
            }
            let ch = |v: f32| v.clamp(0.0, 255.0).round() as u32;
            (a << 24) | (ch(p[0]) << 16) | (ch(p[1]) << 8) | ch(p[2])
        })
        .collect()
}

/// Renderiza o texto em branco sobre preto e devolve a cobertura (0..1) por pixel.
fn render_text(size: i32, text: &str) -> Vec<f32> {
    let n = (size * size) as usize;
    let mut out = vec![0f32; n];
    unsafe {
        let screen = GetDC(HWND::default());
        let dc = CreateCompatibleDC(screen);
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let bmi = bitmap_info(size);
        let Ok(bmp) = CreateDIBSection(dc, &bmi, DIB_RGB_COLORS, &mut bits, None, 0) else {
            let _ = DeleteDC(dc);
            ReleaseDC(HWND::default(), screen);
            return out;
        };
        let old_bmp = SelectObject(dc, HGDIOBJ::from(bmp));

        let digits = text.chars().count();
        let factor = if digits >= 2 { 0.56 } else { 0.62 };
        let font = CreateFontW(
            -((size as f32 * factor).round() as i32),
            0,
            0,
            0,
            FW_BOLD.0 as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET.0 as u32,
            OUT_TT_PRECIS.0 as u32,
            CLIP_DEFAULT_PRECIS.0 as u32,
            ANTIALIASED_QUALITY.0 as u32,
            (DEFAULT_PITCH.0 | FF_SWISS.0) as u32,
            w!("Segoe UI"),
        );
        let old_font = SelectObject(dc, HGDIOBJ::from(font));
        SetBkMode(dc, TRANSPARENT);
        SetTextColor(dc, COLORREF(0x00FF_FFFF));
        let mut wide: Vec<u16> = text.encode_utf16().collect();
        let mut rc = RECT { left: 0, top: 0, right: size, bottom: size };
        DrawTextW(dc, &mut wide, &mut rc, DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX);
        let _ = GdiFlush();

        let px = std::slice::from_raw_parts(bits as *const u32, n);
        for (o, &p) in out.iter_mut().zip(px.iter()) {
            let r = ((p >> 16) & 0xFF) as f32;
            let g = ((p >> 8) & 0xFF) as f32;
            let b = (p & 0xFF) as f32;
            *o = (r.max(g).max(b)) / 255.0;
        }

        SelectObject(dc, old_font);
        SelectObject(dc, old_bmp);
        let _ = DeleteObject(HGDIOBJ::from(font));
        let _ = DeleteObject(HGDIOBJ::from(bmp));
        let _ = DeleteDC(dc);
        ReleaseDC(HWND::default(), screen);
    }
    out
}

fn bitmap_info(size: i32) -> BITMAPINFO {
    BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: size,
            biHeight: -size, // top-down
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    }
}

unsafe fn create_icon(size: i32, bgra: &[u32]) -> HICON {
    let screen = GetDC(HWND::default());
    let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
    let bmi = bitmap_info(size);
    let color = CreateDIBSection(screen, &bmi, DIB_RGB_COLORS, &mut bits, None, 0).unwrap_or_default();
    ReleaseDC(HWND::default(), screen);
    if !bits.is_null() {
        std::ptr::copy_nonoverlapping(bgra.as_ptr(), bits as *mut u32, bgra.len());
    }
    let mask = CreateBitmap(size, size, 1, 1, None);
    let info = ICONINFO {
        fIcon: TRUE,
        xHotspot: 0,
        yHotspot: 0,
        hbmMask: mask,
        hbmColor: color,
    };
    let icon = CreateIconIndirect(&info).unwrap_or_default();
    let _ = DeleteObject(HGDIOBJ::from(color));
    let _ = DeleteObject(HGDIOBJ::from(mask));
    icon
}

#[cfg(test)]
mod tests {
    /// Gera previews ampliados (BMP) em target/icon-preview/, sobre fundo escuro e claro.
    #[test]
    fn preview() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/icon-preview");
        std::fs::create_dir_all(&dir).unwrap();
        let levels = [Some(92u8), Some(45), Some(12), Some(100), None];
        for (light, bg) in [(false, [32u8, 32, 32]), (true, [238u8, 238, 238])] {
            for size in [16, 20, 24, 32] {
                let scale = 8;
                let pad = 2;
                let cell = (size + pad * 2) as usize;
                let w = cell * levels.len() * scale;
                let h = cell * scale;
                let mut img = vec![bg; w * h];
                for (i, lv) in levels.iter().enumerate() {
                    let px = super::render(size, *lv, light);
                    for y in 0..size as usize {
                        for x in 0..size as usize {
                            let p = px[y * size as usize + x];
                            let a = ((p >> 24) & 0xFF) as f32 / 255.0;
                            let c = [((p >> 16) & 0xFF) as f32, ((p >> 8) & 0xFF) as f32, (p & 0xFF) as f32];
                            let o: [u8; 3] = std::array::from_fn(|k| (c[k] * a + bg[k] as f32 * (1.0 - a)) as u8);
                            for sy in 0..scale {
                                for sx in 0..scale {
                                    let ix = (i * cell + pad as usize + x) * scale + sx;
                                    let iy = (pad as usize + y) * scale + sy;
                                    img[iy * w + ix] = o;
                                }
                            }
                        }
                    }
                }
                let name = format!("{}-{}.bmp", if light { "light" } else { "dark" }, size);
                write_bmp(&dir.join(name), w, h, &img);
            }
        }
    }

    fn write_bmp(path: &std::path::Path, w: usize, h: usize, px: &[[u8; 3]]) {
        let row = (w * 3 + 3) & !3;
        let size = 54 + row * h;
        let mut b = Vec::with_capacity(size);
        b.extend_from_slice(b"BM");
        b.extend_from_slice(&(size as u32).to_le_bytes());
        b.extend_from_slice(&[0; 4]);
        b.extend_from_slice(&54u32.to_le_bytes());
        b.extend_from_slice(&40u32.to_le_bytes());
        b.extend_from_slice(&(w as i32).to_le_bytes());
        b.extend_from_slice(&(-(h as i32)).to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&24u16.to_le_bytes());
        b.extend_from_slice(&[0; 24]);
        for y in 0..h {
            for x in 0..w {
                let [r, g, bl] = px[y * w + x];
                b.extend_from_slice(&[bl, g, r]);
            }
            b.extend(std::iter::repeat(0).take(row - w * 3));
        }
        std::fs::write(path, b).unwrap();
    }
}
