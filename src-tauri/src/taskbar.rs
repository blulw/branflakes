//! Windows taskbar thumbnail toolbar: the Previous / Play-Pause / Next
//! buttons shown when hovering the app's taskbar button (`ITaskbarList3`).
//!
//! This is separate from SMTC (`media.rs`): SMTC is the system media flyout
//! and media keys; this is the toolbar under the taskbar hover thumbnail.
//!
//! Design:
//! * The toolbar never plays anything itself. A click is emitted as the same
//!   `media-control` event SMTC uses, so the frontend playback controller
//!   (the single source of truth) decides what happens, filtered through the
//!   resolved capabilities exactly like a media-key press.
//! * The frontend pushes the playback state (`media_update`), including which
//!   of the three actions the active source currently allows. The buttons
//!   mirror that: unsupported actions are shown disabled.
//! * All native work happens on the window's UI thread (COM STA, window
//!   subclass). Other threads only post state through `run_on_main_thread`.
//! * Buttons may be added only once per taskbar button, and the shell
//!   returns S_OK for every call (verified on Windows 11), so success can't
//!   reveal whether buttons exist. We therefore add only when Windows sends
//!   `TaskbarButtonCreated` (first show, Explorer restart: each is a new
//!   button), plus one timer fallback in case that message was delivered
//!   before we subclassed the window. Everything else is an update.

use serde::Serialize;

/// Button ids. Arbitrary, but unlikely to collide with anything else that
/// arrives as `WM_COMMAND` (thumbnail clicks also carry `THBN_CLICKED`).
pub const ID_PREVIOUS: u32 = 0xB0F1;
pub const ID_PLAY_PAUSE: u32 = 0xB0F2;
pub const ID_NEXT: u32 = 0xB0F3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Previous,
    PlayPause,
    Next,
}

impl Action {
    pub fn from_button_id(id: u32) -> Option<Action> {
        match id {
            ID_PREVIOUS => Some(Action::Previous),
            ID_PLAY_PAUSE => Some(Action::PlayPause),
            ID_NEXT => Some(Action::Next),
            _ => None,
        }
    }

    /// The `media-control` action name the frontend controller understands.
    pub fn media_action(self) -> &'static str {
        match self {
            Action::Previous => "previous",
            Action::PlayPause => "toggle",
            Action::Next => "next",
        }
    }
}

/// What the toolbar shows. Derived from the frontend's playback state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub struct ToolbarState {
    pub playing: bool,
    pub can_previous: bool,
    pub can_play_pause: bool,
    pub can_next: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Glyph {
    Previous,
    Play,
    Pause,
    Next,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ButtonSpec {
    pub id: u32,
    pub glyph: Glyph,
    pub tooltip: &'static str,
    pub enabled: bool,
}

/// The three buttons for a given state, in display order.
pub fn button_specs(s: &ToolbarState) -> [ButtonSpec; 3] {
    [
        ButtonSpec { id: ID_PREVIOUS, glyph: Glyph::Previous, tooltip: "Previous", enabled: s.can_previous },
        if s.playing {
            ButtonSpec { id: ID_PLAY_PAUSE, glyph: Glyph::Pause, tooltip: "Pause", enabled: s.can_play_pause }
        } else {
            ButtonSpec { id: ID_PLAY_PAUSE, glyph: Glyph::Play, tooltip: "Play", enabled: s.can_play_pause }
        },
        ButtonSpec { id: ID_NEXT, glyph: Glyph::Next, tooltip: "Next", enabled: s.can_next },
    ]
}

// ------------------------------------------------------------ glyph raster

type Pt = (f32, f32);

fn in_triangle(p: Pt, a: Pt, b: Pt, c: Pt) -> bool {
    let sign = |p1: Pt, p2: Pt, p3: Pt| (p1.0 - p3.0) * (p2.1 - p3.1) - (p2.0 - p3.0) * (p1.1 - p3.1);
    let d1 = sign(p, a, b);
    let d2 = sign(p, b, c);
    let d3 = sign(p, c, a);
    let neg = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
    let pos = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
    !(neg && pos)
}

fn in_rect(p: Pt, x0: f32, y0: f32, x1: f32, y1: f32) -> bool {
    p.0 >= x0 && p.0 <= x1 && p.1 >= y0 && p.1 <= y1
}

/// Is the unit-square point inside the glyph shape?
fn covers(glyph: Glyph, p: Pt) -> bool {
    match glyph {
        Glyph::Play => in_triangle(p, (0.30, 0.18), (0.30, 0.82), (0.82, 0.50)),
        Glyph::Pause => in_rect(p, 0.26, 0.20, 0.42, 0.80) || in_rect(p, 0.58, 0.20, 0.74, 0.80),
        Glyph::Next => in_triangle(p, (0.20, 0.20), (0.20, 0.80), (0.64, 0.50)) || in_rect(p, 0.66, 0.20, 0.79, 0.80),
        Glyph::Previous => in_triangle(p, (0.80, 0.20), (0.80, 0.80), (0.36, 0.50)) || in_rect(p, 0.21, 0.20, 0.34, 0.80),
    }
}

/// Render a glyph as straight-alpha BGRA, top-down rows, 4×4 supersampled.
pub fn render_glyph(glyph: Glyph, size: u32, rgb: (u8, u8, u8)) -> Vec<u8> {
    const SS: u32 = 4;
    let size = size.max(1);
    let mut out = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let mut hits = 0u32;
            for sy in 0..SS {
                for sx in 0..SS {
                    let px = (x as f32 + (sx as f32 + 0.5) / SS as f32) / size as f32;
                    let py = (y as f32 + (sy as f32 + 0.5) / SS as f32) / size as f32;
                    if covers(glyph, (px, py)) {
                        hits += 1;
                    }
                }
            }
            let a = (hits * 255 / (SS * SS)) as u8;
            out.extend_from_slice(&[rgb.2, rgb.1, rgb.0, a]);
        }
    }
    out
}

// ---------------------------------------------------------- tauri glue

/// Cheap, thread-safe handle kept in `AppState`. Deduplicates updates so the
/// UI thread is only touched when the visible state actually changes.
pub struct TaskbarHandle {
    app: tauri::AppHandle,
    last: std::sync::Mutex<Option<ToolbarState>>,
}

impl TaskbarHandle {
    pub fn update(&self, s: ToolbarState) {
        {
            let mut last = self.last.lock().unwrap_or_else(|e| e.into_inner());
            if *last == Some(s) {
                return;
            }
            *last = Some(s);
        }
        #[cfg(windows)]
        {
            if let Err(e) = self.app.run_on_main_thread(move || native::set_state(s)) {
                log::warn!("taskbar toolbar update not delivered: {e}");
            }
        }
        #[cfg(not(windows))]
        let _ = &self.app;
    }
}

/// Attach the toolbar to the main window. Must be called on the UI thread
/// (Tauri's `setup` runs there). Returns `None` when unavailable.
pub fn start(app: &tauri::AppHandle, window: &tauri::WebviewWindow) -> Option<TaskbarHandle> {
    #[cfg(windows)]
    {
        use tauri::Emitter;
        let hwnd = match window.hwnd() {
            Ok(h) => h.0 as isize,
            Err(e) => {
                log::warn!("taskbar toolbar: no window handle: {e}");
                return None;
            }
        };
        let emitter = app.clone();
        let on_click = move |a: Action| {
            let payload = crate::media::MediaControlPayload { action: a.media_action().into(), position_ms: None };
            if let Err(e) = emitter.emit("media-control", payload) {
                log::warn!("taskbar toolbar: could not deliver click: {e}");
            }
        };
        if let Err(e) = native::attach(hwnd, Box::new(on_click)) {
            log::warn!("taskbar toolbar unavailable: {e}");
            return None;
        }
        Some(TaskbarHandle { app: app.clone(), last: std::sync::Mutex::new(None) })
    }
    #[cfg(not(windows))]
    {
        let _ = (app, window);
        None
    }
}

/// Remove the subclass and free native resources. Call on the UI thread at exit.
pub fn shutdown() {
    #[cfg(windows)]
    native::detach();
}

#[cfg(windows)]
pub mod native {
    //! Windows-only implementation. Everything here runs on the UI thread;
    //! state lives in a thread-local so the subclass procedure can reach it.

    use super::{button_specs, render_glyph, Action, Glyph, ToolbarState};
    use std::cell::RefCell;
    use std::rc::Rc;
    use windows::core::w;
    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::Graphics::Gdi::{
        CreateBitmap, CreateDIBSection, DeleteObject, GetDC, ReleaseDC, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HGDIOBJ,
    };
    use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED};
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
    use windows::Win32::UI::Shell::{
        DefSubclassProc, ITaskbarList3, RemoveWindowSubclass, SetWindowSubclass, TaskbarList, THBF_DISABLED, THBF_ENABLED, THBN_CLICKED,
        THB_FLAGS, THB_ICON, THB_TOOLTIP, THUMBBUTTON,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        ChangeWindowMessageFilterEx, CreateIconIndirect, DestroyIcon, GetSystemMetrics, IsWindowVisible, KillTimer, RegisterWindowMessageW, SetTimer,
        HICON, ICONINFO, MSGFLT_ALLOW, SM_CXSMICON, WM_COMMAND, WM_DPICHANGED, WM_NCDESTROY, WM_SETTINGCHANGE, WM_THEMECHANGED, WM_TIMER,
    };

    const SUBCLASS_ID: usize = 0xB0F1_7A5C;
    const FALLBACK_TIMER_ID: usize = 0xB0F1_7A5D;
    const FALLBACK_DELAY_MS: u32 = 3000;

    type ClickFn = Rc<dyn Fn(Action)>;

    struct Icons {
        previous: HICON,
        play: HICON,
        pause: HICON,
        next: HICON,
    }

    impl Icons {
        fn get(&self, g: Glyph) -> HICON {
            match g {
                Glyph::Previous => self.previous,
                Glyph::Play => self.play,
                Glyph::Pause => self.pause,
                Glyph::Next => self.next,
            }
        }
        fn destroy(&self) {
            for h in [self.previous, self.play, self.pause, self.next] {
                if !h.is_invalid() {
                    unsafe {
                        let _ = DestroyIcon(h);
                    }
                }
            }
        }
    }

    struct Toolbar {
        hwnd: HWND,
        list: ITaskbarList3,
        icons: Icons,
        state: ToolbarState,
        added: bool,
        on_click: ClickFn,
        taskbar_created_msg: u32,
        com_initialized: bool,
        /// Number of successful ThumbBarAddButtons calls (diagnostics/tests).
        add_count: u32,
    }

    thread_local! {
        static TOOLBAR: RefCell<Option<Toolbar>> = const { RefCell::new(None) };
    }

    fn light_theme() -> bool {
        let mut value: u32 = 0;
        let mut size = std::mem::size_of::<u32>() as u32;
        let r = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
                w!("SystemUsesLightTheme"),
                RRF_RT_REG_DWORD,
                None,
                Some(&mut value as *mut u32 as *mut _),
                Some(&mut size),
            )
        };
        r.is_ok() && value != 0
    }

    fn make_icon(glyph: Glyph, size: u32, rgb: (u8, u8, u8)) -> windows::core::Result<HICON> {
        let pixels = render_glyph(glyph, size, rgb);
        unsafe {
            let mut bmi = BITMAPINFO::default();
            bmi.bmiHeader = BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: size as i32,
                biHeight: -(size as i32), // top-down
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            };
            let hdc = GetDC(None);
            let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
            let color = CreateDIBSection(Some(hdc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0);
            ReleaseDC(None, hdc);
            let color = color?;
            if bits.is_null() {
                let _ = DeleteObject(HGDIOBJ(color.0));
                return Err(windows::core::Error::from_thread());
            }
            std::ptr::copy_nonoverlapping(pixels.as_ptr(), bits as *mut u8, pixels.len());
            // All-zero AND mask: the colour bitmap's alpha channel decides.
            let mask_bytes = vec![0u8; (size.div_ceil(16) * 2 * size) as usize];
            let mask = CreateBitmap(size as i32, size as i32, 1, 1, Some(mask_bytes.as_ptr() as *const _));
            let info = ICONINFO { fIcon: true.into(), xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: color };
            let icon = CreateIconIndirect(&info);
            let _ = DeleteObject(HGDIOBJ(color.0));
            let _ = DeleteObject(HGDIOBJ(mask.0));
            icon
        }
    }

    fn make_icons() -> windows::core::Result<Icons> {
        let size = unsafe { GetSystemMetrics(SM_CXSMICON) }.clamp(16, 64) as u32;
        let rgb = if light_theme() { (0x1f, 0x1f, 0x1f) } else { (0xff, 0xff, 0xff) };
        Ok(Icons {
            previous: make_icon(Glyph::Previous, size, rgb)?,
            play: make_icon(Glyph::Play, size, rgb)?,
            pause: make_icon(Glyph::Pause, size, rgb)?,
            next: make_icon(Glyph::Next, size, rgb)?,
        })
    }

    fn buttons(tb: &Toolbar) -> [THUMBBUTTON; 3] {
        button_specs(&tb.state).map(|spec| {
            let mut b = THUMBBUTTON {
                dwMask: THB_ICON | THB_TOOLTIP | THB_FLAGS,
                iId: spec.id,
                hIcon: tb.icons.get(spec.glyph),
                dwFlags: if spec.enabled { THBF_ENABLED } else { THBF_DISABLED },
                ..Default::default()
            };
            for (dst, src) in b.szTip.iter_mut().zip(spec.tooltip.encode_utf16()) {
                *dst = src;
            }
            b
        })
    }

    /// Push the current state to the taskbar. `add` = this is a new taskbar
    /// button that has no toolbar yet; otherwise update in place (only once
    /// buttons exist).
    ///
    /// No `RefCell` borrow is held across the COM calls: the shell sends
    /// messages back to the window synchronously, which re-enters
    /// `subclass_proc` on this same thread.
    fn apply_now(add: bool) {
        let job = TOOLBAR.with(|t| {
            t.try_borrow().ok().and_then(|g| g.as_ref().map(|tb| (tb.list.clone(), tb.hwnd, buttons(tb), tb.added)))
        });
        let Some((list, hwnd, b, added)) = job else { return };
        if !add && !added {
            return; // state is stored; it is applied when the buttons get added
        }
        let r = unsafe {
            if add {
                list.ThumbBarAddButtons(hwnd, &b)
            } else {
                list.ThumbBarUpdateButtons(hwnd, &b)
            }
        };
        if let Err(e) = &r {
            log::warn!("taskbar toolbar {} failed: {e}", if add { "add" } else { "update" });
        }
        TOOLBAR.with(|t| {
            if let Ok(mut g) = t.try_borrow_mut() {
                if let Some(tb) = g.as_mut() {
                    if add && r.is_ok() {
                        tb.added = true;
                        tb.add_count += 1;
                        log::info!("taskbar toolbar: buttons added (taskbar button #{})", tb.add_count);
                    }
                }
            }
        });
    }

    unsafe extern "system" fn subclass_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM, _id: usize, _data: usize) -> LRESULT {
        if msg == WM_COMMAND && ((wparam.0 >> 16) & 0xffff) as u32 == THBN_CLICKED {
            let id = (wparam.0 & 0xffff) as u32;
            // Clone the callback out so no borrow is held while it runs.
            let cb = TOOLBAR.with(|t| t.try_borrow().ok().and_then(|g| g.as_ref().map(|tb| tb.on_click.clone())));
            if let (Some(cb), Some(action)) = (cb, Action::from_button_id(id)) {
                cb(action);
                return LRESULT(0);
            }
        } else if msg == WM_NCDESTROY {
            detach();
        } else if msg == WM_THEMECHANGED || msg == WM_SETTINGCHANGE || msg == WM_DPICHANGED {
            refresh_icons();
        } else if msg == WM_TIMER && wparam.0 == FALLBACK_TIMER_ID {
            unsafe {
                let _ = KillTimer(Some(hwnd), FALLBACK_TIMER_ID);
            }
            let pending = TOOLBAR.with(|t| t.try_borrow().ok().and_then(|g| g.as_ref().map(|tb| !tb.added)));
            if pending == Some(true) && unsafe { IsWindowVisible(hwnd) }.as_bool() {
                // We never saw TaskbarButtonCreated (it predates the
                // subclass) but the window has a taskbar button.
                apply_now(true);
            }
            return LRESULT(0);
        } else if msg != 0 {
            let created = TOOLBAR.with(|t| t.try_borrow().ok().and_then(|g| g.as_ref().map(|tb| tb.taskbar_created_msg)));
            if created == Some(msg) {
                // A new taskbar button (first show or Explorer restart):
                // it has no toolbar yet, so add one.
                unsafe {
                    let _ = KillTimer(Some(hwnd), FALLBACK_TIMER_ID);
                }
                apply_now(true);
            }
        }
        unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
    }

    fn refresh_icons() {
        let Ok(icons) = make_icons() else { return };
        let old = TOOLBAR.with(|t| match t.try_borrow_mut() {
            Ok(mut g) => match g.as_mut() {
                Some(tb) => Ok(std::mem::replace(&mut tb.icons, icons)),
                None => Err(icons),
            },
            Err(_) => Err(icons),
        });
        match old {
            Ok(old) => {
                apply_now(false);
                old.destroy();
            }
            Err(unused) => unused.destroy(),
        }
    }

    /// Attach to a window. Idempotent: re-attaching first detaches.
    pub fn attach(hwnd_raw: isize, on_click: Box<dyn Fn(Action)>) -> windows::core::Result<()> {
        detach();
        let hwnd = HWND(hwnd_raw as *mut _);
        unsafe {
            // The UI thread is normally already an STA (WebView2 / OLE); only
            // balance the call if we were the ones to initialise COM.
            let com_initialized = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
            let list: ITaskbarList3 = match CoCreateInstance(&TaskbarList, None, CLSCTX_INPROC_SERVER) {
                Ok(l) => l,
                Err(e) => {
                    if com_initialized {
                        CoUninitialize();
                    }
                    return Err(e);
                }
            };
            list.HrInit()?;
            let icons = make_icons()?;
            let taskbar_created_msg = RegisterWindowMessageW(w!("TaskbarButtonCreated"));
            // Allow the message through UIPI if the app ever runs elevated.
            let _ = ChangeWindowMessageFilterEx(hwnd, taskbar_created_msg, MSGFLT_ALLOW, None);
            let _ = ChangeWindowMessageFilterEx(hwnd, WM_COMMAND, MSGFLT_ALLOW, None);
            TOOLBAR.with(|t| {
                *t.borrow_mut() = Some(Toolbar {
                    hwnd,
                    list,
                    icons,
                    state: ToolbarState::default(),
                    added: false,
                    on_click: Rc::from(on_click),
                    taskbar_created_msg,
                    com_initialized,
                    add_count: 0,
                })
            });
            if !SetWindowSubclass(hwnd, Some(subclass_proc), SUBCLASS_ID, 0).as_bool() {
                detach();
                return Err(windows::core::Error::from_thread());
            }
            // Normally TaskbarButtonCreated arrives once the window shows. If
            // it was already delivered before we subclassed, add on a timer.
            let _ = SetTimer(Some(hwnd), FALLBACK_TIMER_ID, FALLBACK_DELAY_MS, None);
        }
        Ok(())
    }

    pub fn set_state(s: ToolbarState) {
        let changed = TOOLBAR.with(|t| match t.try_borrow_mut() {
            Ok(mut g) => match g.as_mut() {
                Some(tb) if tb.state != s => {
                    tb.state = s;
                    true
                }
                _ => false,
            },
            Err(_) => false,
        });
        if changed {
            apply_now(false);
        }
    }

    /// (added, number of successful adds, current state) — for tests.
    pub fn debug_status() -> Option<(bool, u32, ToolbarState)> {
        TOOLBAR.with(|t| t.try_borrow().ok().and_then(|g| g.as_ref().map(|tb| (tb.added, tb.add_count, tb.state))))
    }

    pub fn detach() {
        let tb = TOOLBAR.with(|t| t.try_borrow_mut().ok().and_then(|mut g| g.take()));
        if let Some(tb) = tb {
            unsafe {
                let _ = KillTimer(Some(tb.hwnd), FALLBACK_TIMER_ID);
                let _ = RemoveWindowSubclass(tb.hwnd, Some(subclass_proc), SUBCLASS_ID);
            }
            tb.icons.destroy();
            let com = tb.com_initialized;
            drop(tb.list);
            if com {
                unsafe { CoUninitialize() };
            }
        }
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buttons_follow_state() {
        let paused = ToolbarState { playing: false, can_previous: true, can_play_pause: true, can_next: false };
        let b = button_specs(&paused);
        assert_eq!(b.iter().map(|b| b.id).collect::<Vec<_>>(), [ID_PREVIOUS, ID_PLAY_PAUSE, ID_NEXT]);
        assert_eq!((b[1].glyph, b[1].tooltip), (Glyph::Play, "Play"));
        assert!(b[0].enabled && b[1].enabled && !b[2].enabled, "unsupported actions are disabled");

        let playing = ToolbarState { playing: true, ..paused };
        let b = button_specs(&playing);
        assert_eq!((b[1].glyph, b[1].tooltip), (Glyph::Pause, "Pause"));

        let nothing = ToolbarState::default();
        assert!(button_specs(&nothing).iter().all(|b| !b.enabled), "no track loaded: everything disabled");
    }

    #[test]
    fn clicks_map_to_existing_media_actions() {
        // The frontend controller already handles these for SMTC.
        assert_eq!(Action::from_button_id(ID_PREVIOUS).unwrap().media_action(), "previous");
        assert_eq!(Action::from_button_id(ID_PLAY_PAUSE).unwrap().media_action(), "toggle");
        assert_eq!(Action::from_button_id(ID_NEXT).unwrap().media_action(), "next");
        assert_eq!(Action::from_button_id(42), None);
    }

    #[test]
    fn glyphs_render_with_alpha() {
        for g in [Glyph::Previous, Glyph::Play, Glyph::Pause, Glyph::Next] {
            let px = render_glyph(g, 16, (255, 255, 255));
            assert_eq!(px.len(), 16 * 16 * 4);
            let opaque = px.chunks(4).filter(|p| p[3] == 255).count();
            let clear = px.chunks(4).filter(|p| p[3] == 0).count();
            assert!(opaque >= 10 && clear > 60, "{g:?}: opaque={opaque} clear={clear}");
        }
        // Play and pause look different.
        assert_ne!(render_glyph(Glyph::Play, 16, (0, 0, 0)), render_glyph(Glyph::Pause, 16, (0, 0, 0)));
    }

    /// Real Windows test: creates a visible window with a taskbar button,
    /// attaches the toolbar, waits for TaskbarButtonCreated, verifies the
    /// buttons are added exactly once (also after a simulated re-creation
    /// message), that a THBN_CLICKED click reaches the callback, and that
    /// detach cleans up. Ignored by default because it briefly shows a window.
    #[cfg(windows)]
    #[test]
    #[ignore = "shows a real window; run with --ignored on a Windows desktop session"]
    fn native_toolbar_on_real_taskbar() {
        use std::cell::Cell;
        use std::rc::Rc;
        use windows::core::w;
        use windows::Win32::Foundation::*;
        use windows::Win32::System::LibraryLoader::GetModuleHandleW;
        use windows::Win32::UI::Shell::THBN_CLICKED;
        use windows::Win32::UI::WindowsAndMessaging::*;

        unsafe extern "system" fn wndproc(h: HWND, m: u32, w: WPARAM, l: LPARAM) -> LRESULT {
            unsafe { DefWindowProcW(h, m, w, l) }
        }
        fn pump(ms: u64) {
            let end = std::time::Instant::now() + std::time::Duration::from_millis(ms);
            let mut msg = MSG::default();
            while std::time::Instant::now() < end {
                unsafe {
                    while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                        let _ = TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }

        unsafe {
            let inst = GetModuleHandleW(None).unwrap();
            let class = w!("BranflakesTaskbarTest");
            let wc = WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: inst.into(), lpszClassName: class, ..Default::default() };
            RegisterClassW(&wc);
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                class,
                w!("branflakes taskbar test"),
                WS_OVERLAPPEDWINDOW,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                300,
                200,
                None,
                None,
                Some(inst.into()),
                None,
            )
            .unwrap();

            let clicks: Rc<Cell<Option<Action>>> = Rc::new(Cell::new(None));
            let c2 = clicks.clone();
            native::attach(hwnd.0 as isize, Box::new(move |a| c2.set(Some(a)))).expect("attach");
            let _ = ShowWindow(hwnd, SW_SHOWMINNOACTIVE); // minimised: buttons must still work
            pump(1000);
            let (added, adds, _) = native::debug_status().unwrap();
            assert!(added, "buttons added on TaskbarButtonCreated");
            assert_eq!(adds, 1);
            // The fallback timer must not add a second time.
            pump(3500);
            assert_eq!(native::debug_status().unwrap().1, 1, "no duplicate add from the fallback timer");

            // State changes are updates, never adds.
            native::set_state(ToolbarState { playing: true, can_previous: true, can_play_pause: true, can_next: true });
            native::set_state(ToolbarState { playing: false, can_previous: false, can_play_pause: true, can_next: true });
            let (_, adds, st) = native::debug_status().unwrap();
            assert_eq!(adds, 1);
            assert!(!st.playing && !st.can_previous);

            // Explorer restart: a new taskbar button announces itself with
            // TaskbarButtonCreated and needs its own toolbar.
            let msg = RegisterWindowMessageW(w!("TaskbarButtonCreated"));
            let _ = SendMessageW(hwnd, msg, None, None);
            assert_eq!(native::debug_status().unwrap().1, 2, "re-added for the recreated taskbar button");

            // A thumbnail button click arrives as WM_COMMAND/THBN_CLICKED.
            let wp = WPARAM(((THBN_CLICKED as usize) << 16) | ID_NEXT as usize);
            let _ = SendMessageW(hwnd, WM_COMMAND, Some(wp), None);
            assert_eq!(clicks.get(), Some(Action::Next));

            let _ = ShowWindow(hwnd, SW_RESTORE);
            pump(300);
            let wp = WPARAM(((THBN_CLICKED as usize) << 16) | ID_PLAY_PAUSE as usize);
            let _ = SendMessageW(hwnd, WM_COMMAND, Some(wp), None);
            assert_eq!(clicks.get(), Some(Action::PlayPause), "works after restore");

            native::detach();
            assert!(native::debug_status().is_none());
            // After detach, clicks are no longer intercepted (and nothing crashes).
            clicks.set(None);
            let _ = SendMessageW(hwnd, WM_COMMAND, Some(wp), None);
            assert_eq!(clicks.get(), None);
            let _ = DestroyWindow(hwnd);
        }
    }
}

