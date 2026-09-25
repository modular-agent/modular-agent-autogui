//! Top-level window lookup, placement and capture. Only Windows is
//! implemented.
//!
//! Positions and sizes are those of the client area, where an application
//! draws its controls: the frame and title bar vary with the theme and DPI,
//! so offsets measured from them would not carry over.

#[cfg(not(target_os = "windows"))]
use enigo::Enigo;
#[cfg(not(target_os = "windows"))]
use modular_agent_core::{Error, PhotonImage, Result};

#[derive(Debug, Clone)]
pub(crate) struct WindowInfo {
    pub id: i64,
    pub title: String,
    /// File name of the executable, such as `notepad.exe`.
    pub process_name: String,
    pub pid: u32,
    /// Client area in physical screen pixels.
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    /// Physical pixels per logical pixel on the window's monitor.
    pub dpi_scale: f64,
}

/// Substrings to match, compared case-insensitively. An empty one matches
/// every window.
#[derive(Debug, Clone)]
pub(crate) struct Query {
    pub title: String,
    pub process_name: String,
}

impl Query {
    fn matches(&self, info: &WindowInfo) -> bool {
        let contains = |s: &str, part: &str| s.to_lowercase().contains(&part.to_lowercase());
        contains(&info.title, &self.title) && contains(&info.process_name, &self.process_name)
    }
}

pub(crate) struct Placement {
    pub activate: bool,
    /// Client size in logical pixels.
    pub size: Option<(f64, f64)>,
}

/// The size in physical pixels differs from the requested one by at most
/// this much through rounding alone.
const SIZE_TOLERANCE: i32 = 1;

pub(crate) fn size_matches(info: &WindowInfo, size: (f64, f64)) -> bool {
    let (w, h) = physical_size(info, size);
    (info.width - w).abs() <= SIZE_TOLERANCE && (info.height - h).abs() <= SIZE_TOLERANCE
}

fn physical_size(info: &WindowInfo, (w, h): (f64, f64)) -> (i32, i32) {
    (
        (w * info.dpi_scale).round() as i32,
        (h * info.dpi_scale).round() as i32,
    )
}

#[cfg(target_os = "windows")]
pub(crate) use imp::{at_point, capture, find, place};

#[cfg(target_os = "windows")]
mod imp {
    use std::ffi::c_void;
    use std::thread;
    use std::time::Duration;

    use enigo::{Direction, Enigo, Key, Keyboard};
    use modular_agent_core::{Error, PhotonImage, Result};
    use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, POINT, RECT};
    use windows::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DwmGetWindowAttribute};
    use windows::Win32::Graphics::Gdi::ClientToScreen;
    use windows::Win32::System::Threading::{
        OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
        QueryFullProcessImageNameW,
    };
    use windows::Win32::UI::HiDpi::GetDpiForWindow;
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GA_ROOT, GetAncestor, GetClientRect, GetForegroundWindow, GetWindowRect,
        GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindow, IsWindowVisible, IsZoomed,
        SW_RESTORE, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOZORDER, SetForegroundWindow, SetWindowPos,
        ShowWindow, WindowFromPoint,
    };
    use windows::core::{BOOL, PWSTR};

    use super::{Placement, Query, WindowInfo, physical_size, size_matches};

    /// Time for an application to lay out its controls again after its
    /// window was restored or resized.
    const RELAYOUT_WAIT: Duration = Duration::from_millis(200);

    /// How long a window may take to become the foreground after the request.
    const ACTIVATE_TIMEOUT: Duration = Duration::from_millis(500);
    const ACTIVATE_POLL: Duration = Duration::from_millis(20);

    /// The topmost visible window matching `query`.
    pub(crate) fn find(query: &Query) -> Result<Option<WindowInfo>> {
        let mut handles: Vec<HWND> = Vec::new();
        unsafe extern "system" fn collect(hwnd: HWND, lparam: LPARAM) -> BOOL {
            // SAFETY: lparam is the address of `handles`, which outlives the
            // EnumWindows call.
            let handles = unsafe { &mut *(lparam.0 as *mut Vec<HWND>) };
            handles.push(hwnd);
            true.into()
        }
        // SAFETY: the callback only runs during this call.
        unsafe { EnumWindows(Some(collect), LPARAM(&mut handles as *mut _ as isize)) }
            .map_err(win_error)?;

        // EnumWindows lists windows from the top of the z-order down.
        for hwnd in handles {
            if !is_shown(hwnd) {
                continue;
            }
            let info = read(hwnd)?;
            if !info.title.is_empty() && query.matches(&info) {
                return Ok(Some(info));
            }
        }
        Ok(None)
    }

    /// The top-level window at a screen point.
    pub(crate) fn at_point(x: i32, y: i32) -> Result<Option<WindowInfo>> {
        // SAFETY: plain Win32 calls with no pointers.
        let hwnd = unsafe { GetAncestor(WindowFromPoint(POINT { x, y }), GA_ROOT) };
        if hwnd.is_invalid() {
            return Ok(None);
        }
        read(hwnd).map(Some)
    }

    /// Restores, resizes and activates the window as asked. Nothing is done
    /// to a window already in the requested state, so no time is spent
    /// waiting for it to lay out again.
    pub(crate) fn place(enigo: &mut Enigo, id: i64, placement: &Placement) -> Result<WindowInfo> {
        let hwnd = HWND(id as *mut c_void);
        // SAFETY: IsWindow accepts any handle value.
        if !unsafe { IsWindow(Some(hwnd)) }.as_bool() {
            return Err(Error::Other("The window has been closed".to_string()));
        }

        let mut changed = false;
        // SAFETY: hwnd is a valid window.
        let (minimized, maximized) = unsafe { (IsIconic(hwnd), IsZoomed(hwnd)) };
        if (minimized.as_bool() && (placement.activate || placement.size.is_some()))
            || (maximized.as_bool() && placement.size.is_some())
        {
            // SAFETY: hwnd is a valid window.
            let _ = unsafe { ShowWindow(hwnd, SW_RESTORE) };
            changed = true;
        }

        if let Some(size) = placement.size {
            let info = read(hwnd)?;
            if !size_matches(&info, size) {
                let (w, h) = physical_size(&info, size);
                let mut frame = RECT::default();
                // SAFETY: hwnd is a valid window and frame is a local.
                unsafe { GetWindowRect(hwnd, &mut frame) }.map_err(win_error)?;
                // Grow the frame by what the client area is missing, which
                // holds for custom frames that AdjustWindowRectEx gets wrong.
                let frame_w = frame.right - frame.left + (w - info.width);
                let frame_h = frame.bottom - frame.top + (h - info.height);
                // SAFETY: hwnd is a valid window.
                unsafe {
                    SetWindowPos(
                        hwnd,
                        None,
                        0,
                        0,
                        frame_w,
                        frame_h,
                        SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
                    )
                }
                .map_err(win_error)?;
                changed = true;
            }
        }

        if placement.activate {
            activate(enigo, hwnd)?;
        }
        if changed {
            thread::sleep(RELAYOUT_WAIT);
        }
        read(hwnd)
    }

    fn activate(enigo: &mut Enigo, hwnd: HWND) -> Result<()> {
        // SAFETY: plain Win32 calls on a valid window.
        unsafe {
            if GetForegroundWindow() == hwnd {
                return Ok(());
            }
            if !SetForegroundWindow(hwnd).as_bool() {
                // Windows lets a process take the foreground only right
                // after it sent input, and an Alt tap counts.
                enigo
                    .key(Key::Alt, Direction::Click)
                    .map_err(|e| Error::Other(format!("Input failed: {e}")))?;
                let _ = SetForegroundWindow(hwnd);
            }
        }
        // The switch to another process's window completes asynchronously,
        // so the foreground may still be the old window right after the call.
        let mut waited = Duration::ZERO;
        loop {
            // SAFETY: plain Win32 call with no arguments.
            if unsafe { GetForegroundWindow() } == hwnd {
                return Ok(());
            }
            if waited >= ACTIVATE_TIMEOUT {
                return Err(Error::Other(
                    "Windows did not let the window come to the front".to_string(),
                ));
            }
            thread::sleep(ACTIVATE_POLL);
            waited += ACTIVATE_POLL;
        }
    }

    fn is_shown(hwnd: HWND) -> bool {
        let mut cloaked = 0u32;
        // SAFETY: cloaked is a local of the size passed.
        let cloaked = unsafe {
            DwmGetWindowAttribute(
                hwnd,
                DWMWA_CLOAKED,
                &mut cloaked as *mut u32 as *mut c_void,
                size_of::<u32>() as u32,
            )
        }
        .is_ok_and(|()| cloaked != 0);
        // SAFETY: IsWindowVisible accepts any handle value.
        unsafe { IsWindowVisible(hwnd) }.as_bool() && !cloaked
    }

    fn read(hwnd: HWND) -> Result<WindowInfo> {
        let mut title = [0u16; 512];
        let mut pid = 0u32;
        let mut client = RECT::default();
        let mut origin = POINT::default();
        // SAFETY: every pointer is to a local that outlives the call.
        let (title_len, dpi) = unsafe {
            let title_len = GetWindowTextW(hwnd, &mut title);
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
            GetClientRect(hwnd, &mut client).map_err(win_error)?;
            let _ = ClientToScreen(hwnd, &mut origin);
            (title_len, GetDpiForWindow(hwnd))
        };
        Ok(WindowInfo {
            id: hwnd.0 as i64,
            title: String::from_utf16_lossy(&title[..title_len.max(0) as usize]),
            process_name: process_name(pid).unwrap_or_default(),
            pid,
            x: origin.x,
            y: origin.y,
            width: client.right - client.left,
            height: client.bottom - client.top,
            dpi_scale: if dpi == 0 { 1.0 } else { dpi as f64 / 96.0 },
        })
    }

    /// Fails for processes this one may not query, such as elevated ones.
    fn process_name(pid: u32) -> Option<String> {
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        // SAFETY: buf and len are locals, and the handle is closed before
        // returning.
        unsafe {
            let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
            let result = QueryFullProcessImageNameW(
                process,
                PROCESS_NAME_WIN32,
                PWSTR(buf.as_mut_ptr()),
                &mut len,
            );
            let _ = CloseHandle(process);
            result.ok()?;
        }
        let path = String::from_utf16_lossy(&buf[..len as usize]);
        path.rsplit('\\').next().map(str::to_string)
    }

    /// Captures the client area as it is drawn, even where other windows
    /// cover it. The image is in physical pixels, so a pixel's position in it
    /// is its offset in the client area.
    pub(crate) fn capture(id: i64) -> Result<(WindowInfo, PhotonImage)> {
        let hwnd = HWND(id as *mut c_void);
        // SAFETY: IsWindow accepts any handle value.
        if !unsafe { IsWindow(Some(hwnd)) }.as_bool() {
            return Err(Error::Other("The window has been closed".to_string()));
        }
        // SAFETY: hwnd is a valid window.
        if unsafe { IsIconic(hwnd) }.as_bool() {
            return Err(Error::Other(
                "The window is minimized; restore it first, such as with Find Window".to_string(),
            ));
        }
        let capture_error = |e: xcap::XCapError| Error::Other(format!("Capture failed: {e}"));
        // xcap identifies a window by the low 32 bits of its handle, which
        // are all a window handle uses.
        let window = xcap::Window::all()
            .map_err(capture_error)?
            .into_iter()
            .find(|w| w.id().is_ok_and(|w_id| w_id == id as u32))
            .ok_or_else(|| Error::Other("The window has been closed".to_string()))?;
        let image = window.capture_image().map_err(capture_error)?;
        let (width, height) = image.dimensions();
        let image = PhotonImage::new(image.into_raw(), width, height);
        Ok((read(hwnd)?, image))
    }

    fn win_error(e: windows::core::Error) -> Error {
        Error::Other(format!("Window operation failed: {e}"))
    }
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn find(_query: &Query) -> Result<Option<WindowInfo>> {
    Err(unsupported())
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn at_point(_x: i32, _y: i32) -> Result<Option<WindowInfo>> {
    Ok(None)
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn place(_enigo: &mut Enigo, _id: i64, _placement: &Placement) -> Result<WindowInfo> {
    Err(unsupported())
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn capture(_id: i64) -> Result<(WindowInfo, PhotonImage)> {
    Err(unsupported())
}

#[cfg(not(target_os = "windows"))]
fn unsupported() -> Error {
    Error::Other("Window operations are only supported on Windows".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(width: i32, height: i32, dpi_scale: f64) -> WindowInfo {
        WindowInfo {
            id: 1,
            title: "Untitled - Notepad".to_string(),
            process_name: "Notepad.exe".to_string(),
            pid: 1,
            x: 0,
            y: 0,
            width,
            height,
            dpi_scale,
        }
    }

    #[test]
    fn size_is_compared_in_physical_pixels() {
        assert!(size_matches(&window(1200, 900, 1.5), (800.0, 600.0)));
        assert!(size_matches(&window(1201, 899, 1.5), (800.0, 600.0)));
        assert!(!size_matches(&window(800, 600, 1.5), (800.0, 600.0)));
    }

    #[test]
    fn query_is_case_insensitive_substring() {
        let w = window(1, 1, 1.0);
        let q = |title: &str, process_name: &str| Query {
            title: title.to_string(),
            process_name: process_name.to_string(),
        };
        assert!(q("notepad", "").matches(&w));
        assert!(q("", "NOTEPAD.EXE").matches(&w));
        assert!(!q("notepad", "code.exe").matches(&w));
    }
}
