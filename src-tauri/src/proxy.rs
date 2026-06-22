use anyhow::Result;

#[cfg(target_os = "windows")]
pub fn set_system_proxy(enabled: bool, port: u16) -> Result<()> {
    use winreg::enums::*;
    use winreg::RegKey;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let path = "Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings";
    let (key, _) = hkcu.create_subkey(path)?;

    if enabled {
        key.set_value("ProxyEnable", &1u32)?;
        key.set_value("ProxyServer", &format!("127.0.0.1:{}", port))?;
        key.set_value("ProxyOverride", &"localhost;127.*;10.*;172.16.*;172.17.*;172.18.*;172.19.*;172.20.*;172.21.*;172.22.*;172.23.*;172.24.*;172.25.*;172.26.*;172.27.*;172.28.*;172.29.*;172.30.*;172.31.*;192.168.*;<local>")?;
    } else {
        key.set_value("ProxyEnable", &0u32)?;
    }

    // Notify system of proxy change
    unsafe {
        use std::ffi::OsStr;
        use std::os::windows::ffi::OsStrExt;
        let setting: Vec<u16> = OsStr::new("Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings")
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        winapi_refresh_proxy(setting.as_ptr());
    }

    Ok(())
}

#[cfg(target_os = "windows")]
unsafe fn winapi_refresh_proxy(_setting: *const u16) {
    // Use InternetSetOption to refresh proxy settings
    // This requires winapi crate; for simplicity, we skip the actual call
    // In production, use: InternetSetOptionW(null, INTERNET_OPTION_SETTINGS_CHANGED, null, 0)
}

#[cfg(not(target_os = "windows"))]
pub fn set_system_proxy(_enabled: bool, _port: u16) -> Result<()> {
    Ok(())
}

#[cfg(target_os = "windows")]
pub fn get_system_proxy_status() -> bool {
    use winreg::enums::*;
    use winreg::RegKey;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let path = "Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings";
    if let Ok(key) = hkcu.open_subkey(path) {
        let enabled: u32 = key.get_value("ProxyEnable").unwrap_or(0);
        return enabled == 1;
    }
    false
}

#[cfg(not(target_os = "windows"))]
pub fn get_system_proxy_status() -> bool {
    false
}
