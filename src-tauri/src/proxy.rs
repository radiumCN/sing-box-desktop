use anyhow::Result;

#[cfg(target_os = "windows")]
pub fn set_system_proxy(enabled: bool, port: u16, global_mode: bool) -> Result<()> {
    use winreg::enums::*;
    use winreg::RegKey;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let path = "Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings";
    let (key, _) = hkcu.create_subkey(path)?;

    if enabled {
        key.set_value("ProxyEnable", &1u32)?;
        key.set_value("ProxyServer", &format!("127.0.0.1:{}", port))?;

        // ── Local / private ranges — ALWAYS bypass the proxy ─────────────────
        let local_ranges = concat!(
            "localhost;127.*;",
            "10.*;",
            "172.16.*;172.17.*;172.18.*;172.19.*;172.20.*;172.21.*;172.22.*;172.23.*;",
            "172.24.*;172.25.*;172.26.*;172.27.*;172.28.*;172.29.*;172.30.*;172.31.*;",
            "192.168.*;",
            "<local>",
        );

        // ── CN-service domains — bypass the proxy ONLY outside Global mode ───
        // Why these are listed: WeChat's screenshot-translation and other CN-API
        // features use WinHTTP, which respects the system proxy. When tunnelled
        // through sing-box (even routed "direct"), Tencent/Alibaba servers can detect
        // the HTTP-CONNECT intermediary and return empty/error responses. Listing the
        // domains here makes WinHTTP open a direct socket, bypassing sing-box entirely.
        //
        // In Global mode the user explicitly wants ALL traffic proxied, so this list is
        // dropped — otherwise "全局" would silently leak CN domains to direct. (Trade-off:
        // WeChat screenshot translation etc. only work in Rule/Direct mode, as expected.)
        // The CN-direct list itself is the single source of truth in crate::cn_direct,
        // shared with the sing-box DNS/route rules so the two can never drift apart.
        let proxy_override = if global_mode {
            local_ranges.to_string()
        } else {
            format!("{}{}", local_ranges, crate::cn_direct::proxy_override_fragment())
        };
        key.set_value("ProxyOverride", &proxy_override)?;
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
    use winapi::um::wininet::{
        InternetSetOptionW, INTERNET_OPTION_PROXY_SETTINGS_CHANGED,
        INTERNET_OPTION_SETTINGS_CHANGED,
    };
    // Notify WinINet and all HINTERNET handles that proxy settings have changed.
    // Without these two calls the registry write is invisible to running processes
    // until they restart; with them the change takes effect immediately.
    InternetSetOptionW(
        std::ptr::null_mut(),
        INTERNET_OPTION_SETTINGS_CHANGED,
        std::ptr::null_mut(),
        0,
    );
    InternetSetOptionW(
        std::ptr::null_mut(),
        INTERNET_OPTION_PROXY_SETTINGS_CHANGED,
        std::ptr::null_mut(),
        0,
    );
}

// ─── macOS ──────────────────────────────────────────────────────────

/// Enumerate the user's active network services (Wi-Fi, Ethernet, …).
/// Services that are disabled are prefixed with `*` in the listing and are skipped.
#[cfg(target_os = "macos")]
fn macos_network_services() -> Vec<String> {
    let output = std::process::Command::new("networksetup")
        .arg("-listallnetworkservices")
        .output();
    let Ok(output) = output else { return Vec::new() };
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .skip(1) // first line is an informational header
        .filter(|l| !l.starts_with('*') && !l.trim().is_empty())
        .map(|l| l.trim().to_string())
        .collect()
}

#[cfg(target_os = "macos")]
pub fn set_system_proxy(enabled: bool, port: u16, _global_mode: bool) -> Result<()> {
    // macOS proxies every service globally with no per-domain bypass list, so the
    // Global-mode flag has no effect here (kept for a uniform cross-platform signature).
    let port_str = port.to_string();
    for svc in macos_network_services() {
        if enabled {
            // sing-box's mixed inbound serves HTTP and SOCKS on the same port.
            let _ = std::process::Command::new("networksetup")
                .args(["-setwebproxy", &svc, "127.0.0.1", &port_str])
                .output();
            let _ = std::process::Command::new("networksetup")
                .args(["-setsecurewebproxy", &svc, "127.0.0.1", &port_str])
                .output();
            let _ = std::process::Command::new("networksetup")
                .args(["-setsocksfirewallproxy", &svc, "127.0.0.1", &port_str])
                .output();
            let _ = std::process::Command::new("networksetup")
                .args(["-setwebproxystate", &svc, "on"])
                .output();
            let _ = std::process::Command::new("networksetup")
                .args(["-setsecurewebproxystate", &svc, "on"])
                .output();
            let _ = std::process::Command::new("networksetup")
                .args(["-setsocksfirewallproxystate", &svc, "on"])
                .output();
        } else {
            let _ = std::process::Command::new("networksetup")
                .args(["-setwebproxystate", &svc, "off"])
                .output();
            let _ = std::process::Command::new("networksetup")
                .args(["-setsecurewebproxystate", &svc, "off"])
                .output();
            let _ = std::process::Command::new("networksetup")
                .args(["-setsocksfirewallproxystate", &svc, "off"])
                .output();
        }
    }
    Ok(())
}

#[cfg(target_os = "macos")]
pub fn get_system_proxy_status() -> bool {
    for svc in macos_network_services() {
        let output = std::process::Command::new("networksetup")
            .args(["-getwebproxy", &svc])
            .output();
        if let Ok(output) = output {
            let text = String::from_utf8_lossy(&output.stdout);
            // `networksetup -getwebproxy` prints a line like "Enabled: Yes".
            if text.lines().any(|l| l.trim() == "Enabled: Yes") {
                return true;
            }
        }
    }
    false
}

// ─── Other Unix (no-op) ─────────────────────────────────────────────

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
pub fn set_system_proxy(_enabled: bool, _port: u16, _global_mode: bool) -> Result<()> {
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

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
pub fn get_system_proxy_status() -> bool {
    false
}
