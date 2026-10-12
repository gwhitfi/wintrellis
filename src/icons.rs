//! App icons for the launcher menu, looked up from what a `[[launcher]]` entry opens.
//! Asking the shell for an icon can take a few milliseconds, so each one is looked up
//! once and cached for the rest of the run.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use windows::Win32::Foundation::SIZE;
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC,
    DeleteObject, GetDIBits, GetObjectW, HBITMAP,
};
use windows::Win32::Storage::FileSystem::SearchPathW;
use windows::Win32::System::Registry::{
    HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RegGetValueW,
};
use windows::Win32::UI::Shell::{
    ASSOCF_NONE, ASSOCSTR_EXECUTABLE, AssocQueryStringW, IShellItemImageFactory,
    SHCreateItemFromParsingName, SIIGBF, SIIGBF_BIGGERSIZEOK, SIIGBF_ICONONLY,
};
use windows::core::{HSTRING, PWSTR, w};

use crate::pixels;

/// An icon's pixels: blue, green, red and premultiplied alpha bytes, top row first
/// (what Direct2D draws). `Rc`: shared, not copied, on every repaint.
pub type Pixels = Rc<[u8]>;

thread_local! {
    /// (target, size in pixels) -> icon, or `None` if it has no icon we could find.
    static CACHE: RefCell<HashMap<(String, i32), Option<Pixels>>> = RefCell::new(HashMap::new());
}

/// The icon of the app `target` opens, `size` × `size` pixels. Needs COM initialized on
/// this thread.
pub fn for_target(target: &str, size: i32) -> Option<Pixels> {
    CACHE.with_borrow_mut(|cache| {
        cache
            .entry((target.to_string(), size))
            .or_insert_with(|| load(target, size))
            .clone()
    })
}

fn load(target: &str, size: i32) -> Option<Pixels> {
    let Some(source) = icon_source(target) else {
        log!("no icon for {target}: couldn't find the program it opens");
        return None;
    };
    let factory: IShellItemImageFactory =
        unsafe { SHCreateItemFromParsingName(&HSTRING::from(&source), None) }.ok()?;
    let get = |flags: SIIGBF| unsafe { factory.GetImage(SIZE { cx: size, cy: size }, flags) };
    // "A bigger one is fine": if the app has no icon this size, the shell hands over a
    // bigger one as is, instead of stretching one with a cheap filter. We shrink it well.
    let image = get(SIIGBF_ICONONLY | SIIGBF_BIGGERSIZEOK)
        .inspect_err(|err| log!("no icon for {target} ({source}): {err}"))
        .ok()?;
    let pixels = sharpen(image, size, target)
        // Smaller than asked for, or unreadable: let the shell stretch it after all.
        .or_else(|| sharpen(get(SIIGBF_ICONONLY).ok()?, size, target))?;
    Some(pixels.into())
}

/// Reads `image`, converts it to premultiplied alpha (what Direct2D expects) and
/// shrinks it to `size` × `size`. `image` is deleted. `None` if it can't be read or is
/// smaller than `size`.
fn sharpen(image: HBITMAP, size: i32, target: &str) -> Option<Vec<u8>> {
    let result = read_pixels(image).and_then(|(mut pixels, w, h)| {
        if w < size || h < size {
            return None;
        }
        let kind = pixels::alpha_kind(&pixels);
        log!("icon for {target}: {w}x{h} ({kind:?} alpha), drawn at {size}x{size}");
        pixels::premultiply(&mut pixels, &kind);
        let (w, h, size) = (w as usize, h as usize, size as usize);
        Some(pixels::shrink(&pixels, w, h, size, size))
    });
    let _ = unsafe { DeleteObject(image.into()) };
    result
}

/// A 32-bit, rows-top-to-bottom description of a `w` × `h` image, for `GetDIBits`.
fn info(w: i32, h: i32) -> BITMAPINFO {
    BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -h, // negative: top row first
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    }
}

/// `bitmap`'s pixels (blue, green, red, alpha; top row first), width and height.
fn read_pixels(bitmap: HBITMAP) -> Option<(Vec<u8>, i32, i32)> {
    let mut header = BITMAP::default();
    let got = unsafe {
        GetObjectW(
            bitmap.into(),
            size_of::<BITMAP>() as i32,
            Some((&mut header as *mut BITMAP).cast()),
        )
    };
    let (w, h) = (header.bmWidth, header.bmHeight.abs());
    if got == 0 || w <= 0 || h <= 0 {
        return None;
    }
    let mut pixels = vec![0u8; (w * h * 4) as usize];
    let mut info = info(w, h);
    let lines = unsafe {
        let dc = CreateCompatibleDC(None);
        // Copies the pixels out in the layout `info` asks for, whatever the bitmap's own.
        let lines = GetDIBits(
            dc,
            bitmap,
            0,
            h as u32,
            Some(pixels.as_mut_ptr().cast()),
            &mut info,
            DIB_RGB_COLORS,
        );
        let _ = DeleteDC(dc);
        lines
    };
    (lines == h).then_some((pixels, w, h))
}

/// What to ask the shell for an icon: paths and `shell:` locations as they are; links
/// ("steam://...") and bare program names ("firefox.exe") resolved to the program
/// that handles them.
fn icon_source(target: &str) -> Option<String> {
    let target = target.trim();
    let is_path = target.contains(['\\', '/']) && !target.contains("://");
    if is_path || target.to_ascii_lowercase().starts_with("shell:") {
        return Some(target.to_string());
    }
    if let Some((scheme, _)) = target.split_once(':') {
        return link_handler(scheme);
    }
    // The same places Win+R looks for a bare program name.
    app_path(target).or_else(|| search_path(target))
}

/// The program Windows runs for links with this scheme, like "steam".
fn link_handler(scheme: &str) -> Option<String> {
    let mut buf = [0u16; 1024];
    let mut len = buf.len() as u32;
    unsafe {
        AssocQueryStringW(
            ASSOCF_NONE,
            ASSOCSTR_EXECUTABLE,
            &HSTRING::from(scheme),
            None,
            Some(PWSTR(buf.as_mut_ptr())),
            &mut len,
        )
    }
    .ok()
    .ok()?;
    // `len` includes the terminating null.
    Some(String::from_utf16_lossy(
        &buf[..(len as usize).saturating_sub(1)],
    ))
}

/// The full path an installer registered for a program name under "App Paths", which
/// Win+R checks first. Per-user entries win over machine-wide ones.
fn app_path(name: &str) -> Option<String> {
    let key = HSTRING::from(format!(
        r"Software\Microsoft\Windows\CurrentVersion\App Paths\{name}"
    ));
    [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE]
        .into_iter()
        .find_map(|root| {
            let mut buf = [0u16; 1024];
            let mut size = size_of_val(&buf) as u32;
            // No value name: the key's default value, which holds the path.
            unsafe {
                RegGetValueW(
                    root,
                    &key,
                    None,
                    RRF_RT_REG_SZ,
                    None,
                    Some(buf.as_mut_ptr().cast()),
                    Some(&mut size),
                )
            }
            .ok()
            .ok()?;
            // `size` is in bytes and includes the terminating null.
            let len = (size as usize / 2).saturating_sub(1);
            // Some installers write the path in quotes.
            let path = String::from_utf16_lossy(&buf[..len]);
            Some(path.trim_matches('"').to_string())
        })
}

/// Finds a program in the PATH, like a terminal does.
fn search_path(name: &str) -> Option<String> {
    let mut buf = [0u16; 1024];
    let len = unsafe { SearchPathW(None, &HSTRING::from(name), w!(".exe"), Some(&mut buf), None) };
    let len = len as usize;
    (len > 0 && len < buf.len()).then(|| String::from_utf16_lossy(&buf[..len]))
}
