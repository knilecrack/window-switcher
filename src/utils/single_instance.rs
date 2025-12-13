use super::to_wstring;

use anyhow::{anyhow, Result};
use windows::core::PCWSTR;
use windows::Win32::{
    Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, HANDLE},
    System::Threading::{CreateEventW, CreateMutexW, ReleaseMutex, SetEvent},
};

pub const RELOAD_CONFIG_EVENT_NAME: &str = "WindowSwitcherReloadConfigEvent";

/// A struct representing one running instance.
pub struct SingleInstance {
    handle: Option<HANDLE>,
}

// SAFETY: SingleInstance only holds a Windows HANDLE which can be safely sent between threads.
// The HANDLE is only accessed from the main thread where the instance is created and dropped.
unsafe impl Send for SingleInstance {}
// SAFETY: SingleInstance's handle is never mutated after creation, only read during drop.
// All operations on the handle are atomic from Windows' perspective.
unsafe impl Sync for SingleInstance {}

impl SingleInstance {
    /// Returns a new SingleInstance object.
    pub fn create(name: &str) -> Result<Self> {
        let name = to_wstring(name);
        let handle = unsafe { CreateMutexW(None, true, PCWSTR(name.as_ptr())) }
            .map_err(|err| anyhow!("Fail to setup single instance, {err}"))?;
        let handle =
            if windows::core::Error::from_thread().code() == ERROR_ALREADY_EXISTS.to_hresult() {
                None
            } else {
                Some(handle)
            };
        Ok(SingleInstance { handle })
    }

    /// Returns whether this instance is single.
    pub fn is_single(&self) -> bool {
        self.handle.is_some()
    }

    /// Signals the running instance to reload its configuration.
    pub fn signal_reload_config() -> Result<()> {
        let event_name = to_wstring(RELOAD_CONFIG_EVENT_NAME);
        let event = unsafe { CreateEventW(None, false, false, PCWSTR(event_name.as_ptr())) }
            .map_err(|err| anyhow!("Failed to open reload config event, {err}"))?;
        unsafe { SetEvent(event) }.map_err(|err| anyhow!("Failed to signal reload config, {err}"))?;
        unsafe { let _ = CloseHandle(event); }
        Ok(())
    }
}

impl Drop for SingleInstance {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            unsafe {
                let _ = ReleaseMutex(handle);
                let _ = CloseHandle(handle);
            }
        }
    }
}
