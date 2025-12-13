use crate::{
    app::{
        WM_USER_SWITCH_APPS, WM_USER_SWITCH_APPS_CANCEL, WM_USER_SWITCH_APPS_DONE,
        WM_USER_SWITCH_WINDOWS, WM_USER_SWITCH_WINDOWS_DONE,
    },
    config::{Hotkey, SWITCH_APPS_HOTKEY_ID, SWITCH_WINDOWS_HOTKEY_ID},
    foreground::IS_FOREGROUND_IN_BLACKLIST,
};

use anyhow::{anyhow, Result};
use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering};
use std::sync::LazyLock;
use windows::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, WPARAM},
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        Input::KeyboardAndMouse::{SCANCODE_LSHIFT, SCANCODE_RSHIFT},
        WindowsAndMessaging::{
            CallNextHookEx, SendMessageW, SetWindowsHookExW, UnhookWindowsHookEx, HHOOK,
            KBDLLHOOKSTRUCT, LLKHF_UP, WH_KEYBOARD_LL,
        },
    },
};

static KEYBOARD_STATE: LazyLock<Mutex<Vec<HotKeyState>>> = LazyLock::new(|| Mutex::new(Vec::new()));
/// Window handle for keyboard hook callbacks. Set once during initialization and never changed.
static WINDOW: AtomicIsize = AtomicIsize::new(0);
/// Tracks whether shift key is currently pressed for reverse switching.
static IS_SHIFT_PRESSED: AtomicBool = AtomicBool::new(false);
/// Tracks the previous keycode to handle modifier release events.
static PREVIOUS_KEYCODE: AtomicU32 = AtomicU32::new(0);

#[derive(Debug)]
pub struct KeyboardListener {
    hook: HHOOK,
}

impl KeyboardListener {
    pub fn init(hwnd: HWND, hotkeys: &[&Hotkey]) -> Result<Self> {
        WINDOW.store(hwnd.0 as isize, Ordering::SeqCst);

        let keyboard_state = hotkeys
            .iter()
            .map(|hotkey| HotKeyState {
                hotkey: (*hotkey).clone(),
                is_modifier_pressed: false,
            })
            .collect();
        *KEYBOARD_STATE.lock() = keyboard_state;

        let hook = unsafe {
            let hinstance = { GetModuleHandleW(None) }
                .map_err(|err| anyhow!("Failed to get module handle, {err}"))?;
            SetWindowsHookExW(
                WH_KEYBOARD_LL,
                Some(keyboard_proc),
                Some(hinstance.into()),
                0,
            )
        }
        .map_err(|err| anyhow!("Failed to set windows hook, {err}"))?;
        info!("keyboard listener start");

        Ok(Self { hook })
    }
}

impl Drop for KeyboardListener {
    fn drop(&mut self) {
        debug!("keyboard listener destroyed");
        if !self.hook.is_invalid() {
            let _ = unsafe { UnhookWindowsHookEx(self.hook) };
        }
    }
}

#[derive(Debug)]
struct HotKeyState {
    hotkey: Hotkey,
    is_modifier_pressed: bool,
}

/// Helper to get the window handle safely from atomic storage.
fn get_window() -> HWND {
    HWND(WINDOW.load(Ordering::SeqCst) as _)
}

unsafe extern "system" fn keyboard_proc(code: i32, w_param: WPARAM, l_param: LPARAM) -> LRESULT {
    // SAFETY: l_param points to a valid KBDLLHOOKSTRUCT provided by Windows
    let kbd_data: &KBDLLHOOKSTRUCT = unsafe { &*(l_param.0 as *const _) };
    debug!("keyboard {kbd_data:?}");
    let mut is_modifier = false;
    let scan_code = kbd_data.scanCode;
    let is_key_pressed = || kbd_data.flags.0 & LLKHF_UP.0 == 0;
    if [SCANCODE_LSHIFT, SCANCODE_RSHIFT].contains(&scan_code) {
        IS_SHIFT_PRESSED.store(is_key_pressed(), Ordering::SeqCst);
    }
    let window = get_window();
    for state in KEYBOARD_STATE.lock().iter_mut() {
        if state.hotkey.modifier.contains(&scan_code) {
            is_modifier = true;
            if is_key_pressed() {
                state.is_modifier_pressed = true;
            } else {
                state.is_modifier_pressed = false;
                if PREVIOUS_KEYCODE.load(Ordering::SeqCst) == state.hotkey.code {
                    let id = state.hotkey.id;
                    if id == SWITCH_APPS_HOTKEY_ID {
                        // SAFETY: window is a valid HWND set during init
                        unsafe { SendMessageW(window, WM_USER_SWITCH_APPS_DONE, None, None) };
                    } else if id == SWITCH_WINDOWS_HOTKEY_ID {
                        // SAFETY: window is a valid HWND set during init
                        unsafe { SendMessageW(window, WM_USER_SWITCH_WINDOWS_DONE, None, None) };
                    }
                }
            }
        }
    }
    if !is_modifier {
        for state in KEYBOARD_STATE.lock().iter_mut() {
            if is_key_pressed() && state.is_modifier_pressed {
                let id = state.hotkey.id;
                if scan_code == state.hotkey.code {
                    let reverse = if IS_SHIFT_PRESSED.load(Ordering::SeqCst) { 1 } else { 0 };
                    if id == SWITCH_APPS_HOTKEY_ID {
                        // SAFETY: window is a valid HWND set during init
                        unsafe {
                            SendMessageW(window, WM_USER_SWITCH_APPS, None, Some(LPARAM(reverse)))
                        };
                        PREVIOUS_KEYCODE.store(scan_code, Ordering::SeqCst);
                        return LRESULT(1);
                    } else if id == SWITCH_WINDOWS_HOTKEY_ID && !IS_FOREGROUND_IN_BLACKLIST.load(Ordering::SeqCst) {
                        // SAFETY: window is a valid HWND set during init
                        unsafe {
                            SendMessageW(
                                window,
                                WM_USER_SWITCH_WINDOWS,
                                None,
                                Some(LPARAM(reverse)),
                            )
                        };
                        PREVIOUS_KEYCODE.store(scan_code, Ordering::SeqCst);
                        return LRESULT(1);
                    }
                } else if scan_code == 0x01 && id == SWITCH_APPS_HOTKEY_ID {
                    // SAFETY: window is a valid HWND set during init
                    unsafe { SendMessageW(window, WM_USER_SWITCH_APPS_CANCEL, None, None) };
                    PREVIOUS_KEYCODE.store(scan_code, Ordering::SeqCst);
                    return LRESULT(1);
                }
            }
        }
    }
    // SAFETY: CallNextHookEx is called with valid parameters from the hook chain
    unsafe { CallNextHookEx(None, code, w_param, l_param) }
}
