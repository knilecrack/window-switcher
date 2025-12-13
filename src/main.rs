#![windows_subsystem = "windows"]

use anyhow::{anyhow, Result};
use std::{
    fs::{File, OpenOptions},
    path::Path,
};

use window_switcher::{alert, load_config, start, utils::SingleInstance};

fn main() {
    if let Err(err) = run() {
        alert!("{err}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let config = load_config().unwrap_or_default();
    if let Some(log_file) = &config.log_file {
        let file = prepare_log_file(log_file).map_err(|err| {
            anyhow!(
                "Failed to prepare log file at {}, {err}",
                log_file.display()
            )
        })?;
        simple_logging::log_to(file, config.log_level);
    }
    
    let instance = SingleInstance::create("WindowSwitcherMutex")?;
    if !instance.is_single() {
        // Another instance is running, signal it to reload config
        SingleInstance::signal_reload_config()?;
        return Ok(());
    }

    start(&config)
}

fn prepare_log_file(path: &Path) -> std::io::Result<File> {
    if path.exists() {
        OpenOptions::new().append(true).open(path)
    } else {
        File::create(path)
    }
}
