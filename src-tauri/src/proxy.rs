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
        // ProxyOverride — addresses/domains in this list bypass the proxy entirely.
        //
        // Why Tencent/WeChat domains are listed here:
        // WeChat's screenshot-translation and other CN-API features use WinHTTP,
        // which respects the system proxy. When those requests are tunnelled through
        // sing-box (even when sing-box routes them "direct"), Tencent's servers can
        // detect the HTTP-CONNECT proxy intermediary via connection metadata and
        // return empty / error responses. Listing the domains here makes WinHTTP
        // open a direct TCP socket to Tencent without going through sing-box at all,
        // which is the most reliable fix for WeChat screenshot translation and
        // similar CN-service features that break under a local proxy.
        key.set_value("ProxyOverride", &concat!(
            // ── Local / private ranges ──────────────────────────────────
            "localhost;127.*;",
            "10.*;",
            "172.16.*;172.17.*;172.18.*;172.19.*;172.20.*;172.21.*;172.22.*;172.23.*;",
            "172.24.*;172.25.*;172.26.*;172.27.*;172.28.*;172.29.*;172.30.*;172.31.*;",
            "192.168.*;",
            "<local>;",
            // ── Tencent / WeChat ─────────────────────────────────────────
            // WeChat PC screenshot translation calls these domains directly.
            "*.qq.com;*.weixin.qq.com;",
            "*.tencent.com;*.tencentcloudapi.com;",
            "*.qcloud.com;*.myqcloud.com;",
            "*.wechat.com;*.weixin.com;",
            "*.gtimg.cn;*.qpic.cn;*.tenpay.com;",
            // ── Other major CN services ───────────────────────────────────
            "*.taobao.com;*.tmall.com;*.alicdn.com;*.alipay.com;",
            "*.alibaba.com;*.aliyun.com;*.aliyuncs.com;",
            "*.baidu.com;*.bdstatic.com;",
            "*.163.com;*.126.net;*.netease.com;",
            "*.bilibili.com;*.bilivideo.com;",
            "*.jd.com;*.jdcdn.com;",
            "*.weibo.com;*.sina.com;",
            "*.bytedance.com;*.douyin.com;*.toutiao.com;",
            "*.feishu.cn;*.feishu.com"
        ))?;
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
pub fn set_system_proxy(enabled: bool, port: u16) -> Result<()> {
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

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
pub fn get_system_proxy_status() -> bool {
    false
}
