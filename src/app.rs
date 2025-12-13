use crate::config::{edit_config_file, Config};
use crate::foreground::ForegroundWatcher;
use crate::keyboard::KeyboardListener;
use crate::painter::{find_clicked_app_index, GdiAAPainter};
use crate::startup::Startup;
use crate::trayicon::TrayIcon;
use crate::utils::{
    check_error, get_app_icon, get_foreground_window, get_window_user_data, is_iconic_window,
    is_running_as_admin, list_windows, set_foreground_window, set_window_user_data,
    RELOAD_CONFIG_EVENT_NAME,
};

use anyhow::{anyhow, Result};
use indexmap::IndexSet;
use std::collections::HashMap;
use windows::core::{w, PCWSTR};
use windows::Win32::{
    Foundation::{GetLastError, HINSTANCE, HWND, LPARAM, LRESULT, WPARAM},
    System::LibraryLoader::GetModuleHandleW,
    UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, GetWindowLongPtrW,
        LoadCursorW, PostMessageW, PostQuitMessage, RegisterClassW, RegisterWindowMessageW,
        SetWindowLongPtrW, TranslateMessage, CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, GWL_STYLE,
        HICON, HTCLIENT, IDC_ARROW, MSG, WINDOW_STYLE, WM_COMMAND, WM_ERASEBKGND, WM_LBUTTONUP,
        WM_NCHITTEST, WM_RBUTTONUP, WNDCLASSW, WS_CAPTION, WS_EX_LAYERED, WS_EX_TOOLWINDOW,
        WS_EX_TOPMOST,
    },
};

pub const NAME: PCWSTR = w!("Window Switcher");
pub const WM_USER_TRAYICON: u32 = 6000;
pub const WM_USER_REGISTER_TRAYICON: u32 = 6001;
pub const WM_USER_SWITCH_APPS: u32 = 6010;
pub const WM_USER_SWITCH_APPS_DONE: u32 = 6011;
pub const WM_USER_SWITCH_APPS_CANCEL: u32 = 6012;
pub const WM_USER_SWITCH_WINDOWS: u32 = 6020;
pub const WM_USER_SWITCH_WINDOWS_DONE: u32 = 6021;
pub const WM_USER_RELOAD_CONFIG: u32 = 6030;
pub const IDM_EXIT: u32 = 1;
pub const IDM_STARTUP: u32 = 2;
pub const IDM_CONFIGURE: u32 = 3;

pub fn start(config: &Config) -> Result<()> {
    info!("start config={config:?}");
    App::start(config)
}

/// Listen to this message to recreate the tray icon since the taskbar has been recreated.
/// Uses AtomicU32 for thread-safe access to the dynamically registered message ID.
static WM_TASKBARCREATED: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

pub struct App {
    hwnd: HWND,
    is_admin: bool,
    trayicon: Option<TrayIcon>,
    startup: Startup,
    config: Config,
    switch_windows_state: SwitchWindowsState,
    switch_apps_state: Option<SwitchAppsState>,
    cached_icons: HashMap<String, HICON>,
    painter: GdiAAPainter,
}

impl App {
    pub fn start(config: &Config) -> Result<()> {
        let hwnd = Self::create_window()?;
        let painter = GdiAAPainter::new(hwnd)?;

        let _foreground_watcher = ForegroundWatcher::init(&config.switch_windows_blacklist)?;
        let _keyboard_listener = KeyboardListener::init(hwnd, &config.to_hotkeys())?;

        let trayicon = match config.trayicon {
            true => Some(TrayIcon::create()),
            false => None,
        };

        let is_admin = is_running_as_admin()?;
        debug!("is_admin {is_admin}");

        let startup = Startup::init(is_admin)?;

        let mut app = App {
            hwnd,
            is_admin,
            trayicon,
            startup,
            config: config.clone(),
            switch_windows_state: SwitchWindowsState {
                cache: None,
                modifier_released: true,
            },
            switch_apps_state: None,
            cached_icons: Default::default(),
            painter,
        };

        app.set_trayicon();

        // SAFETY: We store the App in user data to be retrieved by window_proc callbacks.
        // The pointer remains valid for the lifetime of the window and is properly
        // deallocated when IDM_EXIT is triggered via Box::from_raw.
        let app_ptr = Box::into_raw(Box::new(app)) as _;

        check_error(|| set_window_user_data(hwnd, app_ptr))
            .map_err(|err| anyhow!("Failed to set window ptr, {err}"))?;

        // Start the reload config event listener
        Self::start_reload_config_listener(hwnd)?;

        Self::eventloop()
    }

    fn start_reload_config_listener(hwnd: HWND) -> Result<()> {
        use crate::utils::to_wstring;
        use windows::Win32::Foundation::{HANDLE, WAIT_OBJECT_0};
        use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject, INFINITE};

        let event_name = to_wstring(RELOAD_CONFIG_EVENT_NAME);
        let event = unsafe { CreateEventW(None, false, false, PCWSTR(event_name.as_ptr())) }
            .map_err(|err| anyhow!("Failed to create reload config event, {err}"))?;

        let hwnd_ptr = hwnd.0 as isize;
        let event_ptr = event.0 as isize;
        std::thread::spawn(move || {
            let event = HANDLE(event_ptr as _);
            loop {
                let result = unsafe { WaitForSingleObject(event, INFINITE) };
                if result == WAIT_OBJECT_0 {
                    let _ = unsafe {
                        PostMessageW(
                            Some(HWND(hwnd_ptr as _)),
                            WM_USER_RELOAD_CONFIG,
                            WPARAM(0),
                            LPARAM(0),
                        )
                    };
                }
            }
        });

        Ok(())
    }

    fn eventloop() -> Result<()> {
        let mut message = MSG::default();
        loop {
            let ret = unsafe { GetMessageW(&mut message, None, 0, 0) };
            match ret.0 {
                -1 => {
                    unsafe { GetLastError() }.ok()?;
                }
                0 => break,
                _ => unsafe {
                    let _ = TranslateMessage(&message);
                    DispatchMessageW(&message);
                },
            }
        }

        Ok(())
    }

    fn create_window() -> Result<HWND> {
        // Register taskbar created message for tray icon recreation
        let taskbar_created_msg = unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) };
        WM_TASKBARCREATED.store(taskbar_created_msg, std::sync::atomic::Ordering::SeqCst);

        let hinstance = unsafe { GetModuleHandleW(None) }
            .map_err(|err| anyhow!("Failed to get current module handle, {err}"))?;

        let hcursor = unsafe { LoadCursorW(None, IDC_ARROW) }
            .map_err(|err| anyhow!("Failed to load arrow cursor, {err}"))?;

        let window_class = WNDCLASSW {
            hCursor: hcursor,
            hInstance: HINSTANCE(hinstance.0),
            lpszClassName: NAME,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(App::window_proc),
            ..Default::default()
        };

        let atom = check_error(|| unsafe { RegisterClassW(&window_class) })
            .map_err(|err| anyhow!("Failed to register class, {err}"))?;

        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
                PCWSTR(atom as _),
                NAME,
                WINDOW_STYLE(0),
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                None,
                None,
                Some(hinstance.into()),
                None,
            )
        }
        .map_err(|err| anyhow!("Failed to create windows, {err}"))?;

        // hide caption
        let mut style = unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) } as u32;
        style &= !WS_CAPTION.0;
        unsafe { SetWindowLongPtrW(hwnd, GWL_STYLE, style as _) };

        Ok(hwnd)
    }

    fn set_trayicon(&mut self) {
        if let Some(trayicon) = self.trayicon.as_mut() {
            match trayicon.register(self.hwnd) {
                Ok(()) => {
                    info!("trayicon registered");
                    let _ = trayicon.show_balloon(
                        "Window Switcher",
                        "Ready to switch! Your windows are now under control 🪟",
                    );
                }
                Err(err) => {
                    if !trayicon.exist() {
                        error!("{err}, retrying in 3 second");
                        let hwnd = self.hwnd.0 as isize;
                        std::thread::spawn(move || {
                            std::thread::sleep(std::time::Duration::from_secs(3));
                            let _ = unsafe {
                                PostMessageW(
                                    Some(HWND(hwnd as _)),
                                    WM_USER_REGISTER_TRAYICON,
                                    WPARAM(0),
                                    LPARAM(0),
                                )
                            };
                        });
                    }
                }
            }
        }
    }

    /// Window procedure callback for handling Windows messages.
    /// 
    /// # Safety
    /// This function is marked unsafe extern "system" because it's called by Windows
    /// as a callback. Windows guarantees valid parameters when calling this function.
    unsafe extern "system" fn window_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match Self::handle_message(hwnd, msg, wparam, lparam) {
            Ok(ret) => ret,
            Err(err) => {
                error!("{err}");
                unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
            }
        }
    }

    fn handle_message(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> Result<LRESULT> {
        match msg {
            WM_USER_TRAYICON => {
                let app = get_app(hwnd)?;
                if let Some(trayicon) = app.trayicon.as_mut() {
                    let keycode = lparam.0 as u32;
                    if keycode == WM_LBUTTONUP || keycode == WM_RBUTTONUP {
                        trayicon.show(app.startup.is_enable)?;
                    }
                }
                return Ok(LRESULT(0));
            }
            WM_USER_SWITCH_APPS => {
                debug!("message WM_USER_SWITCH_APPS");
                let app = get_app(hwnd)?;
                let reverse = lparam.0 == 1;
                app.switch_apps(reverse)?;
                if let Some(state) = &app.switch_apps_state {
                    app.painter.paint(state);
                }
            }
            WM_USER_SWITCH_APPS_DONE => {
                debug!("message WM_USER_SWITCH_APPS_DONE");
                let app = get_app(hwnd)?;
                app.do_switch_app();
            }
            WM_USER_SWITCH_APPS_CANCEL => {
                debug!("message WM_USER_SWITCH_APPS_CANCEL");
                let app = get_app(hwnd)?;
                app.cancel_switch_app();
            }
            WM_USER_SWITCH_WINDOWS => {
                debug!("message WM_USER_SWITCH_WINDOWS");
                let app = get_app(hwnd)?;
                let reverse = lparam.0 == 1;
                let hwnd = app
                    .switch_apps_state
                    .as_ref()
                    .and_then(|state| state.apps.get(state.index).map(|(_, id)| *id))
                    .unwrap_or_else(get_foreground_window);
                app.switch_windows(hwnd, reverse)?;
            }
            WM_USER_SWITCH_WINDOWS_DONE => {
                debug!("message WM_USER_SWITCH_WINDOWS_DONE");
                let app = get_app(hwnd)?;
                app.switch_windows_state.modifier_released = true;
            }
            WM_USER_RELOAD_CONFIG => {
                debug!("message WM_USER_RELOAD_CONFIG");
                let app = get_app(hwnd)?;
                app.reload_config();
            }
            WM_NCHITTEST => {
                return Ok(LRESULT(HTCLIENT as _));
            }
            WM_LBUTTONUP => {
                let app = get_app(hwnd)?;
                app.click();
            }
            WM_COMMAND => {
                let value = wparam.0 as u32;
                let kind = ((value >> 16) & 0xffff) as u16;
                let id = value & 0xffff;
                if kind == 0 {
                    match id {
                        IDM_EXIT => {
                            if let Ok(app) = get_app(hwnd) {
                                // SAFETY: app was created via Box::into_raw in start(), and this
                                // is the only place where Box::from_raw is called to reclaim ownership.
                                // After this drop, the window is destroyed via PostQuitMessage.
                                unsafe { drop(Box::from_raw(app)) }
                            }
                            // SAFETY: PostQuitMessage terminates the message loop cleanly
                            unsafe { PostQuitMessage(0) }
                        }
                        IDM_STARTUP => {
                            let app = get_app(hwnd)?;
                            app.startup.toggle()?;
                        }
                        IDM_CONFIGURE => {
                            if let Err(err) = edit_config_file() {
                                alert!("{err}");
                            }
                        }
                        _ => {}
                    }
                }
            }
            WM_ERASEBKGND => {
                return Ok(LRESULT(0));
            }
            _ if msg == WM_USER_REGISTER_TRAYICON || msg == WM_TASKBARCREATED.load(std::sync::atomic::Ordering::SeqCst) => {
                let app = get_app(hwnd)?;
                app.set_trayicon();
            }
            _ => {}
        }
        // SAFETY: DefWindowProcW is called with valid window parameters
        Ok(unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) })
    }

    fn switch_windows(&mut self, hwnd: HWND, reverse: bool) -> Result<bool> {
        let windows = list_windows(
            self.config.switch_windows_ignore_minimal,
            self.config.switch_windows_only_current_desktop(),
            self.is_admin,
        )?;
        debug!(
            "switch windows: hwnd:{hwnd:?} reverse:{reverse} state:{:?}",
            self.switch_windows_state
        );
        let module_path = match windows
            .iter()
            .find(|(_, v)| v.iter().any(|(id, _)| *id == hwnd))
            .map(|(k, _)| k.clone())
        {
            Some(v) => v,
            None => return Ok(false),
        };
        match windows.get(&module_path) {
            None => Ok(false),
            Some(windows) => {
                let windows_len = windows.len();
                if windows_len == 1 {
                    return Ok(false);
                }
                let current_id = windows[0].0;
                let mut index = 1;
                let mut state_id = current_id;
                let mut state_windows = vec![];
                if windows_len > 2 {
                    if let Some((cache_module_path, cache_id, cache_index, cache_windows)) =
                        self.switch_windows_state.cache.as_ref()
                    {
                        if cache_module_path == &module_path {
                            if self.switch_windows_state.modifier_released {
                                if *cache_id != current_id {
                                    if let Some((i, _)) =
                                        windows.iter().enumerate().find(|(_, (v, _))| v == cache_id)
                                    {
                                        index = i;
                                    }
                                }
                            } else {
                                state_id = *cache_id;
                                let mut windows_set: IndexSet<isize> =
                                    windows.iter().map(|(v, _)| v.0 as _).collect();
                                for id in cache_windows {
                                    if windows_set.contains(id) {
                                        state_windows.push(*id);
                                        windows_set.swap_remove(id);
                                    }
                                }
                                state_windows.extend(windows_set);
                                index = if reverse {
                                    if *cache_index == 0 {
                                        windows_len - 1
                                    } else {
                                        cache_index - 1
                                    }
                                } else if *cache_index >= windows_len - 1 {
                                    0
                                } else {
                                    cache_index + 1
                                };
                            }
                        }
                    }
                }
                if state_windows.is_empty() {
                    state_windows = windows.iter().map(|(v, _)| v.0 as _).collect();
                }
                // Use get() for bounds-safe access to prevent potential panic
                let hwnd = state_windows
                    .get(index)
                    .map(|v| HWND(*v as _))
                    .unwrap_or_else(|| HWND(state_windows[0] as _));
                self.switch_windows_state = SwitchWindowsState {
                    cache: Some((module_path.clone(), state_id, index, state_windows)),
                    modifier_released: false,
                };
                set_foreground_window(hwnd);

                Ok(true)
            }
        }
    }

    fn switch_apps(&mut self, reverse: bool) -> Result<()> {
        debug!(
            "switch apps: reverse:{reverse}, state:{:?}",
            self.switch_apps_state
        );
        if let Some(state) = self.switch_apps_state.as_mut() {
            if reverse {
                if state.index == 0 {
                    state.index = state.apps.len() - 1;
                } else {
                    state.index -= 1;
                }
            } else if state.index == state.apps.len() - 1 {
                state.index = 0;
            } else {
                state.index += 1;
            };
            debug!("switch apps: new index:{}", state.index);
            return Ok(());
        }
        let windows = list_windows(
            self.config.switch_apps_ignore_minimal,
            self.config.switch_apps_only_current_desktop(),
            self.is_admin,
        )?;
        let mut apps = vec![];
        for (module_path, hwnds) in windows.iter() {
            // hwnds is guaranteed to be non-empty by list_windows implementation
            let module_hwnd = if hwnds.is_empty() {
                continue;
            } else if is_iconic_window(hwnds[0].0) {
                hwnds.last().map(|(hwnd, _)| *hwnd).unwrap_or(hwnds[0].0)
            } else {
                hwnds[0].0
            };
            let module_hicon = self
                .cached_icons
                .entry(module_path.clone())
                .or_insert_with(|| {
                    get_app_icon(
                        &self.config.switch_apps_override_icons,
                        module_path,
                        module_hwnd,
                    )
                });
            apps.push((*module_hicon, module_hwnd));
        }
        let num_apps = apps.len() as i32;
        if num_apps == 0 {
            return Ok(());
        }

        let index = if apps.len() == 1 {
            0
        } else if reverse {
            apps.len() - 1
        } else {
            1
        };

        let state = SwitchAppsState { apps, index };
        self.switch_apps_state = Some(state);
        debug!("switch apps, new state:{:?}", self.switch_apps_state);
        Ok(())
    }

    fn click(&mut self) {
        if let Some(state) = self.switch_apps_state.as_mut() {
            if let Some(i) = find_clicked_app_index(state) {
                state.index = i;
                self.do_switch_app();
            }
        }
    }

    fn do_switch_app(&mut self) {
        if let Some(state) = self.switch_apps_state.take() {
            if let Some((_, id)) = state.apps.get(state.index) {
                set_foreground_window(*id);
            }
            self.painter.unpaint(state);
        }
    }

    fn cancel_switch_app(&mut self) {
        if let Some(state) = self.switch_apps_state.take() {
            self.painter.unpaint(state);
        }
    }

    fn reload_config(&mut self) {
        use crate::load_config;
        info!("reloading configuration");
        match load_config() {
            Ok(new_config) => {
                self.config = new_config;
                info!("configuration reloaded successfully");
                if let Some(trayicon) = self.trayicon.as_mut() {
                    if let Err(err) = trayicon.show_balloon("Window Switcher", "Configuration reloaded") {
                        error!("Failed to show balloon notification: {err}");
                    }
                } else {
                    // Fallback to message box if trayicon is disabled
                    alert!("Configuration reloaded");
                }
            }
            Err(err) => {
                error!("Failed to reload configuration: {err}");
                alert!("Failed to reload configuration: {err}");
            }
        }
    }
}

/// Retrieves the App instance stored in window user data.
/// 
/// # Safety
/// This function is safe to call as long as:
/// - The hwnd is the window created by App::create_window()
/// - The App pointer was stored via set_window_user_data() in start()
/// - The App has not been deallocated (only happens on IDM_EXIT)
fn get_app(hwnd: HWND) -> Result<&'static mut App> {
    let ptr = check_error(|| get_window_user_data(hwnd))
        .map_err(|err| anyhow!("Failed to get window ptr, {err}"))?;
    if ptr == 0 {
        return Err(anyhow!("Window user data is null"));
    }
    // SAFETY: ptr was set via Box::into_raw in start() and points to valid App memory
    // until IDM_EXIT triggers deallocation. The 'static lifetime is valid because
    // the App lives for the duration of the program.
    let app: &'static mut App = unsafe { &mut *(ptr as *mut App) };
    Ok(app)
}

#[derive(Debug)]
struct SwitchWindowsState {
    cache: Option<(String, HWND, usize, Vec<isize>)>,
    modifier_released: bool,
}

#[derive(Debug)]
pub struct SwitchAppsState {
    pub apps: Vec<(HICON, HWND)>,
    pub index: usize,
}
