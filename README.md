# Window Switcher

Window-Switcher offers hotkeys for quickly switching windows on Windows OS:

1. ```Alt+`(Backtick)```: switch between windows of the same app.

![switch-windows](https://github.com/sigoden/window-switcher/assets/4012553/06d387ce-31fd-450b-adf3-01bfcfc4bce3)

1. ```Alt+Tab```: switch between apps. (disabled by default)

![switch-apps](https://github.com/sigoden/window-switcher/assets/4012553/0c74a7ca-3a48-4458-8d2d-b40dc041f067)

**💡 Hold down the `Alt` key and tap the ``` `(Backtick)/Tab ``` key to cycle through windows/apps, Press ```Alt + `(Backtick)/Tab``` and release both keys to switch to the last active window/app.**

## Installation

1. **Download:** Visit the [Github Release](https://github.com/sigoden/windows-switcher/releases) and download the `windows-switcher.zip` file.
2. **Extract:** Unzip the downloaded file and extract the `window-switcher.exe` to your preferred location.
3. **Launch:** `window-switcher.exe` is a standalone executable, no installation is required, just double-click the file to run it.

For the tech-savvy, here's a one-liner to automate the installation:

```ps1
iwr -useb https://raw.githubusercontent.com/sigoden/window-switcher/main/install.ps1 | iex
```

## Configuration

Window-Switcher offers various customization options to tailor its behavior to your preferences. You can define custom keyboard shortcuts, enable or disable specific features, and fine-tune settings through a configuration file.

### Configuration File Location

Window-Switcher looks for the configuration file `window-switcher.ini` in the following locations (in order of preference):

1. **User Config Folder** (Recommended): `%LOCALAPPDATA%\WindowSwitcher\window-switcher.ini`
   - On most systems: `C:\Users\[YourUsername]\AppData\Local\WindowSwitcher\window-switcher.ini`
   - Falls back to: `%APPDATA%\WindowSwitcher\window-switcher.ini` if LOCALAPPDATA is not available

2. **Application Directory**: Same directory as `window-switcher.exe`

The application will automatically create the config folder if it doesn't exist when you first save a configuration. This approach allows for user-specific settings while maintaining backward compatibility with existing installations.

### Configuration Options

Once you've made changes to the configuration, the changes will take effect automatically without needing to restart Window-Switcher.

Here is the default configuration:

```ini
# Whether to show trayicon, yes/no
trayicon = yes 

[switch-windows]

# Hotkey to switch windows
hotkey = alt+`

# List of hotkey conflict apps
# e.g. game1.exe,game2.exe
blacklist =

# Ignore minimal windows
ignore_minimal = no

# Only switch within the current virtual desktops: yes/no/auto
only_current_desktop = auto

[switch-apps]

# Whether to enable switching apps
enable = no 

# Hotkey to switch apps
hotkey = alt+tab

# Ignore minimal windows
ignore_minimal = no

# Only switch apps within the current virtual desktops: yes/no/auto
only_current_desktop = auto
```

## Running as Administrator (Optional)

The window-switcher works in standard user mode. But only the window-switcher running in administrator mode can manage applications running in administrator mode.

**Important:** If you enable the startup option while running in standard user mode, it will launch in standard mode upon system reboot. To ensure startup with admin privileges, launch the window-switcher as administrator first before enabling startup.

## License

Copyright (c) 2023-2025 window-switcher developers.

window-switcher is made available under the terms of the MIT License, at your option.

See the LICENSE files for license details.
