//! The speaker volume, for the workspace bar, through Windows' Core Audio API (COM):
//! the default output device's level and mute, and notices when either changes or the
//! default device itself changes (e.g. headphones plugged in). Windows reports those
//! on its own threads; we only post a message to the main thread's listener window,
//! which re-reads.

use std::cell::RefCell;

use windows::Win32::Foundation::{HWND, LPARAM, PROPERTYKEY, WPARAM};
use windows::Win32::Media::Audio::Endpoints::{
    IAudioEndpointVolume, IAudioEndpointVolumeCallback, IAudioEndpointVolumeCallback_Impl,
};
use windows::Win32::Media::Audio::{
    AUDIO_VOLUME_NOTIFICATION_DATA, DEVICE_STATE, EDataFlow, ERole, IMMDeviceEnumerator,
    IMMNotificationClient, IMMNotificationClient_Impl, MMDeviceEnumerator, eConsole, eRender,
};
use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};
use windows::core::{PCWSTR, Result, implement};

/// Posted to the listener window (see `display.rs`) when the volume or mute changed (`wParam` = `CHANGED`), or
/// the default output device did (`NEW_DEVICE`).
pub const WM_VOLUME: u32 = WM_APP + 5;
pub const CHANGED: usize = 0;
pub const NEW_DEVICE: usize = 1;

/// What we hold on to while running.
struct Audio {
    /// Lists audio devices and tells us when the default one changes.
    devices: IMMDeviceEnumerator,
    /// The default output device's volume control; `None` without one.
    endpoint: Option<IAudioEndpointVolume>,
    /// Our listeners, kept so they can be unregistered.
    on_change: IAudioEndpointVolumeCallback,
    on_device: IMMNotificationClient,
}

thread_local! {
    static AUDIO: RefCell<Option<Audio>> = const { RefCell::new(None) };
}

/// Windows calls this (on one of its threads) when the volume or mute changes.
#[implement(IAudioEndpointVolumeCallback)]
struct VolumeChanged {
    /// The listener window to post to. A number, not an `HWND`: this object is used on
    /// other threads, and Rust only lets plain data cross threads.
    window: isize,
}

impl IAudioEndpointVolumeCallback_Impl for VolumeChanged_Impl {
    fn OnNotify(&self, _data: *mut AUDIO_VOLUME_NOTIFICATION_DATA) -> Result<()> {
        post(self.window, CHANGED);
        Ok(())
    }
}

/// Windows calls this (on one of its threads) when audio devices change. Only a new
/// default output device matters to us.
#[implement(IMMNotificationClient)]
struct DeviceChanged {
    window: isize,
}

impl IMMNotificationClient_Impl for DeviceChanged_Impl {
    fn OnDefaultDeviceChanged(&self, flow: EDataFlow, role: ERole, _id: &PCWSTR) -> Result<()> {
        // Output (not microphones), for normal use: the device the volume icon shows.
        if flow == eRender && role == eConsole {
            post(self.window, NEW_DEVICE);
        }
        Ok(())
    }

    fn OnDeviceStateChanged(&self, _id: &PCWSTR, _state: DEVICE_STATE) -> Result<()> {
        Ok(())
    }

    fn OnDeviceAdded(&self, _id: &PCWSTR) -> Result<()> {
        Ok(())
    }

    fn OnDeviceRemoved(&self, _id: &PCWSTR) -> Result<()> {
        Ok(())
    }

    fn OnPropertyValueChanged(&self, _id: &PCWSTR, _key: &PROPERTYKEY) -> Result<()> {
        Ok(())
    }
}

/// Tells the main thread something changed; it re-reads the volume itself.
fn post(window: isize, what: usize) {
    let window = HWND(window as *mut _);
    let _ = unsafe { PostMessageW(Some(window), WM_VOLUME, WPARAM(what), LPARAM(0)) };
}

/// Connects to the default output device and starts listening for changes, posting
/// notices to `listener`. Call once, on the main thread, after COM is initialized.
pub fn start(listener: HWND) -> Result<()> {
    let window = listener.0 as isize;
    let devices: IMMDeviceEnumerator =
        unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }?;
    let on_device: IMMNotificationClient = DeviceChanged { window }.into();
    unsafe { devices.RegisterEndpointNotificationCallback(&on_device) }?;
    let on_change: IAudioEndpointVolumeCallback = VolumeChanged { window }.into();
    AUDIO.set(Some(Audio {
        devices,
        endpoint: None,
        on_change,
        on_device,
    }));
    use_default_device();
    Ok(())
}

/// Switches to the current default output device: at start, and when it changes.
pub fn use_default_device() {
    AUDIO.with_borrow_mut(|audio| {
        let Some(audio) = audio else {
            return;
        };
        if let Some(old) = audio.endpoint.take() {
            let _ = unsafe { old.UnregisterControlChangeNotify(&audio.on_change) };
        }
        let endpoint = unsafe {
            audio
                .devices
                .GetDefaultAudioEndpoint(eRender, eConsole)
                .and_then(|device| device.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None))
        };
        match endpoint {
            Ok(endpoint) => {
                let _ = unsafe { endpoint.RegisterControlChangeNotify(&audio.on_change) };
                audio.endpoint = Some(endpoint);
            }
            // E.g. no speakers or headphones at all: the icon shows muted.
            Err(err) => log!("no audio output device: {err}"),
        }
    });
}

/// The volume (0.0 silent to 1.0 full) and whether it's muted; `None` without an
/// output device.
pub fn level() -> Option<(f32, bool)> {
    AUDIO.with_borrow(|audio| {
        let endpoint = audio.as_ref()?.endpoint.as_ref()?;
        unsafe {
            let level = endpoint.GetMasterVolumeLevelScalar().ok()?;
            let muted = endpoint.GetMute().ok()?.as_bool();
            Some((level, muted))
        }
    })
}

/// Turns the volume up (`by` > 0) or down by `by` (0.02 = 2%), staying between 0 and
/// 100%. Turning it up while muted unmutes, as Windows' own slider does. Windows then
/// reports the change like any other (`WM_VOLUME`), which redraws the bar.
pub fn change(by: f32) {
    AUDIO.with_borrow(|audio| {
        let Some(endpoint) = audio.as_ref().and_then(|a| a.endpoint.as_ref()) else {
            return;
        };
        unsafe {
            let Ok(level) = endpoint.GetMasterVolumeLevelScalar() else {
                return;
            };
            // No event context: we don't need to recognize our own change later.
            let _ = endpoint.SetMasterVolumeLevelScalar(stepped(level, by), std::ptr::null());
            if by > 0.0 {
                let _ = endpoint.SetMute(false, std::ptr::null());
            }
        }
    });
}

/// `level` changed by `by`, on whole percents (so steps never drift to 41.999%), kept
/// between 0.0 and 1.0.
fn stepped(level: f32, by: f32) -> f32 {
    ((level * 100.0).round() + by * 100.0)
        .round()
        .clamp(0.0, 100.0)
        / 100.0
}

/// Stops listening (on exit).
pub fn stop() {
    if let Some(audio) = AUDIO.take() {
        unsafe {
            if let Some(endpoint) = &audio.endpoint {
                let _ = endpoint.UnregisterControlChangeNotify(&audio.on_change);
            }
            let _ = audio
                .devices
                .UnregisterEndpointNotificationCallback(&audio.on_device);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_land_on_whole_percents_within_range() {
        assert_eq!(stepped(0.5, 0.02), 0.52);
        assert_eq!(stepped(0.419_99, 0.02), 0.44); // 42% + 2%
        assert_eq!(stepped(0.01, -0.02), 0.0); // not below silent
        assert_eq!(stepped(0.99, 0.04), 1.0); // not above full
    }
}
