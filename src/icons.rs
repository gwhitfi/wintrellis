//! App icons for the launcher menu, looked up from what a `[[launcher]]` entry opens.
//! Asking the shell for an icon can take a few milliseconds, so each one is looked up
//! once and cached for the rest of the run.

use std::cell::RefCell;
use std::collections::HashMap;

use windows::Win32::Foundation::SIZE;
use windows::Win32::Graphics::Gdi::HBITMAP;
use windows::Win32::Storage::FileSystem::SearchPathW;
use windows::Win32::System::Registry::{
    HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RegGetValueW,
};
use windows::Win32::UI::Shell::{
    ASSOCF_NONE, ASSOCSTR_EXECUTABLE, AssocQueryStringW, IShellItemImageFactory,
    SHCreateItemFromParsingName, SIIGBF_ICONONLY,
};
use windows::core::{HSTRING, PWSTR, w};

thread_local! {
    /// (target, size in pixels) -> icon, or `None` if it has no icon we could find.
    static CACHE: RefCell<HashMap<(String, i32), Option<HBITMAP>>> = RefCell::new(HashMap::new());
}

/// The icon of the app `target` opens, `size` pixels square, as a 32-bit bitmap with
/// transparency (what menu items take). Needs COM initialized on this thread.
pub fn for_target(target: &str, size: i32) -> Option<HBITMAP> {
    CACHE.with_borrow_mut(|cache| {
        *cache
            .entry((target.to_string(), size))
            .or_insert_with(|| load(target, size))
    })
}

fn load(target: &str, size: i32) -> Option<HBITMAP> {
    let Some(source) = icon_source(target) else {
        log!("no icon for {target}: couldn't find the program it opens");
        return None;
    };
    let image = unsafe {
        let factory: IShellItemImageFactory =
            SHCreateItemFromParsingName(&HSTRING::from(&source), None).ok()?;
        factory.GetImage(SIZE { cx: size, cy: size }, SIIGBF_ICONONLY)
    };
    image
        .inspect_err(|err| log!("no icon for {target} ({source}): {err}"))
        .ok()
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
