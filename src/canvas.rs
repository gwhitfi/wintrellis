//! Smooth drawing for our own see-through windows. Direct2D draws anti-aliased shapes
//! and DirectWrite text into an off-screen 32-bit image whose pixels each carry their own
//! transparency; `UpdateLayeredWindow` then hands that image to Windows as the window's
//! whole look. Unlike a color key, edge pixels can be partly see-through, so curves and
//! text look smooth against whatever is behind the window.
//!
//! `draw_on` is the same smooth drawing for ordinary (solid) windows, into the DC they
//! paint with.

use std::cell::RefCell;

use windows::Win32::Foundation::{COLORREF, HWND, POINT, RECT, SIZE};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE, D2D1_ALPHA_MODE_IGNORE, D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F,
    D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_RENDER_TARGET_PROPERTIES,
    D2D1_RENDER_TARGET_TYPE_SOFTWARE, D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE, D2D1CreateFactory,
    ID2D1DCRenderTarget, ID2D1Factory, ID2D1RenderTarget,
};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL,
    DWRITE_FONT_WEIGHT, DWRITE_PARAGRAPH_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_CENTER,
    DWRITE_TEXT_METRICS, DWRITE_TRIMMING, DWRITE_TRIMMING_GRANULARITY_CHARACTER,
    DWRITE_WORD_WRAPPING_NO_WRAP, DWriteCreateFactory, IDWriteFactory, IDWriteFontCollection,
    IDWriteTextFormat,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION,
    CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, HDC,
    SelectObject,
};
use windows::Win32::UI::WindowsAndMessaging::{ULW_ALPHA, UpdateLayeredWindow};
use windows::core::{Result, w};

use crate::layout::Rect;

thread_local! {
    /// Made on first use and kept: creating it loads Direct2D. Rebuilt after a failure
    /// (e.g. the graphics driver restarted).
    static TARGET: RefCell<Option<ID2D1DCRenderTarget>> = const { RefCell::new(None) };

    /// Like `TARGET`, for solid windows (`draw_on`).
    static SOLID: RefCell<Option<ID2D1DCRenderTarget>> = const { RefCell::new(None) };

    /// Made on first use and kept: creating it loads DirectWrite.
    static TEXT: RefCell<Option<IDWriteFactory>> = const { RefCell::new(None) };
}

/// Runs `draw` on a fully see-through `at.w` × `at.h` canvas and shows the result as
/// `hwnd`'s whole look, at `at` on screen. `hwnd` must be layered (`WS_EX_LAYERED`) and
/// never given `SetLayeredWindowAttributes`, which switches a window to the other way of
/// being see-through. Coordinates in `draw` are pixels from the canvas's top-left corner.
pub fn show(
    hwnd: HWND,
    at: Rect,
    draw: impl FnOnce(&ID2D1RenderTarget) -> Result<()>,
) -> Result<()> {
    let target = target()?;
    unsafe {
        // The off-screen image: 32 bits per pixel (blue, green, red, alpha), selected into
        // a memory DC so Direct2D can draw into it and Windows can read it.
        let dc = CreateCompatibleDC(None);
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: at.w,
                // Negative: rows go top to bottom, like the screen.
                biHeight: -at.h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        let bitmap = match CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0) {
            Ok(bitmap) => bitmap,
            Err(err) => {
                let _ = DeleteDC(dc);
                return Err(err);
            }
        };
        let old = SelectObject(dc, bitmap.into());
        let result = render(&target, dc, at, draw).and_then(|()| present(hwnd, dc, at));
        SelectObject(dc, old);
        let _ = DeleteObject(bitmap.into());
        let _ = DeleteDC(dc);
        if result.is_err() {
            TARGET.set(None);
        }
        result
    }
}

/// Runs `draw` on `dc`, the `w` × `h` drawing surface of an ordinary solid window (not
/// layered), such as its paint buffer. Text gets ClearType, which needs the solid
/// background. Drawn by the CPU: a small window takes about a millisecond, and the
/// graphics driver is never loaded into WinTrellis.
pub fn draw_on(
    dc: HDC,
    w: i32,
    h: i32,
    draw: impl FnOnce(&ID2D1RenderTarget) -> Result<()>,
) -> Result<()> {
    let target = match SOLID.with_borrow(|t| t.clone()) {
        Some(target) => target,
        None => {
            let target = make_target(D2D1_ALPHA_MODE_IGNORE)?;
            SOLID.set(Some(target.clone()));
            target
        }
    };
    let result = render(&target, dc, Rect { x: 0, y: 0, w, h }, draw);
    if result.is_err() {
        SOLID.set(None);
    }
    result
}

/// Points Direct2D at `dc`'s image and draws into it.
fn render(
    target: &ID2D1DCRenderTarget,
    dc: HDC,
    at: Rect,
    draw: impl FnOnce(&ID2D1RenderTarget) -> Result<()>,
) -> Result<()> {
    let whole = RECT {
        left: 0,
        top: 0,
        right: at.w,
        bottom: at.h,
    };
    unsafe {
        target.BindDC(dc, &whole)?;
        target.BeginDraw();
        // All zeros: every pixel fully see-through until something is drawn on it.
        target.Clear(Some(&D2D1_COLOR_F::default()));
        let drawn = draw(target);
        // EndDraw is where Direct2D actually draws, and where its errors show up.
        let ended = target.EndDraw(None, None);
        drawn.and(ended)
    }
}

/// Hands the finished image to Windows as `hwnd`'s look, using each pixel's alpha.
fn present(hwnd: HWND, dc: HDC, at: Rect) -> Result<()> {
    let blend = BLENDFUNCTION {
        BlendOp: AC_SRC_OVER as u8,
        BlendFlags: 0,
        SourceConstantAlpha: 255,
        // Use each pixel's own transparency.
        AlphaFormat: AC_SRC_ALPHA as u8,
    };
    unsafe {
        UpdateLayeredWindow(
            hwnd,
            None,
            Some(&POINT { x: at.x, y: at.y }),
            Some(&SIZE { cx: at.w, cy: at.h }),
            Some(dc),
            Some(&POINT::default()),
            COLORREF(0),
            Some(&blend),
            ULW_ALPHA,
        )
    }
}

/// The Direct2D target for see-through windows (`show`), made once.
fn target() -> Result<ID2D1DCRenderTarget> {
    if let Some(target) = TARGET.with_borrow(|t| t.clone()) {
        return Ok(target);
    }
    // Colors pre-multiplied by alpha, which is what UpdateLayeredWindow expects.
    let target = make_target(D2D1_ALPHA_MODE_PREMULTIPLIED)?;
    // ClearType text needs an opaque background; on a see-through canvas it smears.
    unsafe { target.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE) };
    TARGET.set(Some(target.clone()));
    Ok(target)
}

/// A Direct2D target that draws into a GDI DC: blue, green, red, alpha pixels, with
/// alpha used as `alpha` says. Drawn by the CPU: our windows are small, and letting
/// Direct2D pick the GPU loads the graphics driver into WinTrellis (measured: an AMD
/// driver; 17 MB at startup instead of 4) for no visible gain.
fn make_target(alpha: D2D1_ALPHA_MODE) -> Result<ID2D1DCRenderTarget> {
    let factory: ID2D1Factory =
        unsafe { D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None) }?;
    let properties = D2D1_RENDER_TARGET_PROPERTIES {
        r#type: D2D1_RENDER_TARGET_TYPE_SOFTWARE,
        pixelFormat: D2D1_PIXEL_FORMAT {
            format: DXGI_FORMAT_B8G8R8A8_UNORM,
            alphaMode: alpha,
        },
        // 96 means one Direct2D unit is one pixel; callers scale for the monitor.
        dpiX: 96.0,
        dpiY: 96.0,
        ..Default::default()
    };
    unsafe { factory.CreateDCRenderTarget(&properties) }
}

/// Text centered (both ways) in the rectangle it's drawn into: Windows 11's UI font,
/// `size` pixels tall, at `weight`.
pub fn text_format(size: f32, weight: DWRITE_FONT_WEIGHT) -> Result<IDWriteTextFormat> {
    unsafe {
        let format = text_factory()?.CreateTextFormat(
            w!("Segoe UI Variable Text"),
            None::<&IDWriteFontCollection>, // the fonts installed in Windows
            weight,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            size,
            w!("en-us"),
        )?;
        format.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
        format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
        Ok(format)
    }
}

/// Makes `format` keep text on one line, cut off with "…" where it doesn't fit.
pub fn one_line(format: &IDWriteTextFormat) -> Result<()> {
    let trimming = DWRITE_TRIMMING {
        granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER,
        delimiter: 0,
        delimiterCount: 0,
    };
    unsafe {
        format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
        let ellipsis = text_factory()?.CreateEllipsisTrimmingSign(format)?;
        format.SetTrimming(&trimming, &ellipsis)
    }
}

/// How wide `text` is in pixels in `format`, spaces at the end included (so a text
/// cursor after them lands in the right place).
pub fn text_width(text: &str, format: &IDWriteTextFormat) -> Result<f32> {
    let wide: Vec<u16> = text.encode_utf16().collect();
    let mut metrics = DWRITE_TEXT_METRICS::default();
    unsafe {
        // A box far wider than any text, but not f32::MAX: centered text sits in the
        // middle of the box, and the middle of an infinite box breaks the measurement.
        let layout = text_factory()?.CreateTextLayout(&wide, format, 100_000.0, 100_000.0)?;
        layout.GetMetrics(&mut metrics)?;
    }
    Ok(metrics.widthIncludingTrailingWhitespace)
}

/// DirectWrite, made on first use and kept.
fn text_factory() -> Result<IDWriteFactory> {
    if let Some(factory) = TEXT.with_borrow(|f| f.clone()) {
        return Ok(factory);
    }
    let factory: IDWriteFactory = unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED) }?;
    TEXT.set(Some(factory.clone()));
    Ok(factory)
}

/// A Windows color (0x00BBGGRR) as Direct2D wants it: red, green, blue and `alpha`
/// (0.0 see-through to 1.0 solid), each from 0.0 to 1.0.
pub fn color(c: COLORREF, alpha: f32) -> D2D1_COLOR_F {
    let channel = |shift: u32| ((c.0 >> shift) & 0xFF) as f32 / 255.0;
    D2D1_COLOR_F {
        r: channel(0),
        g: channel(8),
        b: channel(16),
        a: alpha,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors_convert_channel_by_channel() {
        // 0x00BBGGRR: blue 0x00, green 0x80, red 0xFF (orange-ish).
        let c = color(COLORREF(0x0000_80FF), 0.5);
        assert_eq!((c.r, c.b, c.a), (1.0, 0.0, 0.5));
        assert!((c.g - 128.0 / 255.0).abs() < 1e-6);
    }
}
