//! Smooth drawing for our own see-through windows. Direct2D draws anti-aliased shapes
//! and DirectWrite text into an off-screen 32-bit image whose pixels each carry their own
//! transparency; `UpdateLayeredWindow` then hands that image to Windows as the window's
//! whole look. Unlike a color key, edge pixels can be partly see-through, so curves and
//! text look smooth against whatever is behind the window.

use std::cell::RefCell;

use windows::Win32::Foundation::{COLORREF, HWND, POINT, RECT, SIZE};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_RENDER_TARGET_PROPERTIES,
    D2D1_RENDER_TARGET_TYPE_DEFAULT, D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE, D2D1CreateFactory,
    ID2D1DCRenderTarget, ID2D1Factory, ID2D1RenderTarget,
};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL,
    DWRITE_FONT_WEIGHT, DWRITE_PARAGRAPH_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_CENTER,
    DWriteCreateFactory, IDWriteFactory, IDWriteFontCollection, IDWriteTextFormat,
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

/// The Direct2D target that draws into a GDI DC, made once.
fn target() -> Result<ID2D1DCRenderTarget> {
    if let Some(target) = TARGET.with_borrow(|t| t.clone()) {
        return Ok(target);
    }
    let factory: ID2D1Factory =
        unsafe { D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None) }?;
    let properties = D2D1_RENDER_TARGET_PROPERTIES {
        r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
        // Matches the image: blue, green, red, alpha; colors pre-multiplied by alpha,
        // which is what UpdateLayeredWindow expects.
        pixelFormat: D2D1_PIXEL_FORMAT {
            format: DXGI_FORMAT_B8G8R8A8_UNORM,
            alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
        },
        // 96 means one Direct2D unit is one pixel; callers scale for the monitor.
        dpiX: 96.0,
        dpiY: 96.0,
        ..Default::default()
    };
    let target = unsafe { factory.CreateDCRenderTarget(&properties) }?;
    // ClearType text needs an opaque background; on a see-through canvas it smears.
    unsafe { target.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE) };
    TARGET.set(Some(target.clone()));
    Ok(target)
}

/// Text centered (both ways) in the rectangle it's drawn into: Windows 11's UI font,
/// `size` pixels tall, at `weight`.
pub fn text_format(size: f32, weight: DWRITE_FONT_WEIGHT) -> Result<IDWriteTextFormat> {
    let factory = match TEXT.with_borrow(|f| f.clone()) {
        Some(factory) => factory,
        None => {
            let factory: IDWriteFactory =
                unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED) }?;
            TEXT.set(Some(factory.clone()));
            factory
        }
    };
    unsafe {
        let format = factory.CreateTextFormat(
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
