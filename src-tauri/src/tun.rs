use anyhow::{Result, anyhow};
use std::path::PathBuf;

/// Check if the current process is running as Administrator
#[cfg(target_os = "windows")]
pub fn is_elevated() -> bool {
    use std::mem;
    use winapi::um::processthreadsapi::OpenProcessToken;
    use winapi::um::processthreadsapi::GetCurrentProcess;
    use winapi::um::securitybaseapi::GetTokenInformation;
    use winapi::um::winnt::{TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY, HANDLE};

    unsafe {
        let mut token: HANDLE = mem::zeroed();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return false;
        }
        let mut elevation: TOKEN_ELEVATION = mem::zeroed();
        let mut size = mem::size_of::<TOKEN_ELEVATION>() as u32;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            &mut elevation as *mut _ as *mut _,
            size,
            &mut size,
        );
        if ok == 0 {
            return false;
        }
        elevation.TokenIsElevated != 0
    }
}

#[cfg(not(target_os = "windows"))]
pub fn is_elevated() -> bool {
    false
}

/// Relaunch the current process with UAC elevation (Windows only)
#[cfg(target_os = "windows")]
pub fn relaunch_as_admin() -> Result<()> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use winapi::um::shellapi::ShellExecuteW;
    use winapi::um::winuser::SW_SHOWNORMAL;

    let exe = std::env::current_exe()?;
    let exe_wide: Vec<u16> = OsStr::new(exe.to_str().unwrap_or(""))
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let verb: Vec<u16> = OsStr::new("runas")
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            exe_wide.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };

    // ShellExecuteW returns > 32 on success
    if result as usize > 32 {
        // Exit current non-elevated instance
        std::process::exit(0);
    } else {
        Err(anyhow!("UAC 提权请求被拒绝或失败（错误码: {}）", result as usize))
    }
}

#[cfg(not(target_os = "windows"))]
pub fn relaunch_as_admin() -> Result<()> {
    Err(anyhow!("不支持此平台"))
}

/// Clean up WinTun state before starting sing-box in TUN mode.
///
/// Two distinct failure modes can block a TUN start, each with its own fix:
///
///   1. Orphaned adapter — a previous crash left a "sing-box-tun*" network adapter behind.
///      Symptom: "Cannot create a file when that file already exists. | ... Element not found."
///      Fix: Remove-NetAdapter (network-stack level, safe). Combined with the unique
///      interface name we now generate per start, this class of error is fully avoided.
///
///   2. Stale wintun driver service — the HKLM\...\Services\wintun service entry got wedged
///      (e.g. after a Windows update or a hard kill).
///      Symptom: "configure tun interface: The system cannot find the file specified."
///      Fix: `sc.exe delete wintun`. sing-box re-installs the driver automatically on the
///      next WintunCreateAdapter call, restoring a clean RUNNING state.
///
/// NOTE: We deliberately do NOT touch the Network-Adapter class registry keys
/// ({4D36E972-...}). Editing those can corrupt the driver and is exactly what caused the
/// regression from "already exists" to "cannot find the file specified".
///
/// Must be called with admin privileges (the app always runs elevated).
#[cfg(target_os = "windows")]
pub async fn cleanup_stale_tun_adapter() {
    use tokio::process::Command as TokioCommand;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let ps = r#"
# Remove any leftover sing-box TUN network adapters (safe, network-stack level)
Get-NetAdapter -Name 'sing-box-tun*' -ErrorAction SilentlyContinue |
    Remove-NetAdapter -Confirm:$false -ErrorAction SilentlyContinue

# Clear a wedged wintun driver service so sing-box can re-register it cleanly.
# This repairs the "The system cannot find the file specified" failure; sing-box
# reinstalls the driver automatically on the next adapter creation.
$svc = Get-Service -Name 'wintun' -ErrorAction SilentlyContinue
if ($svc) {
    & sc.exe stop wintun | Out-Null
    & sc.exe delete wintun | Out-Null
}
"#;

    let _ = TokioCommand::new("powershell")
        .args(["-NonInteractive", "-NoProfile", "-WindowStyle", "Hidden", "-Command", ps])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .await;

    // Brief pause so Windows propagates the removal before sing-box re-creates the adapter.
    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
}

#[cfg(not(target_os = "windows"))]
pub async fn cleanup_stale_tun_adapter() {}

/// Check if WinTun driver DLL is present alongside sing-box binary
pub fn wintun_available() -> bool {
    let bin_dir = crate::updater::singbox_binary_path()
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));

    // WinTun ships wintun.dll in the same directory as sing-box on Windows
    bin_dir.join("wintun.dll").exists()
}

/// Download WinTun driver DLL
/// WinTun is bundled inside some sing-box releases; if missing, download from wintun.net
pub async fn download_wintun(dest_dir: &std::path::Path) -> Result<()> {
    // Official WinTun zip download (amd64 build)
    let url = "https://www.wintun.net/builds/wintun-0.14.1.zip";
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .user_agent("sing-box-win/0.1.0")
        .build()?;

    let resp = client.get(url).send().await?;
    if !resp.status().is_success() {
        return Err(anyhow!("WinTun 下载失败: HTTP {}", resp.status()));
    }
    let zip_bytes = resp.bytes().await?;

    // Ensure destination directory exists
    std::fs::create_dir_all(dest_dir)
        .map_err(|e| anyhow!("无法创建目录 {:?}: {}", dest_dir, e))?;

    // Extract wintun/bin/amd64/wintun.dll
    use std::io::Cursor;
    let cursor = Cursor::new(zip_bytes.as_ref());
    let mut archive = zip::ZipArchive::new(cursor)?;

    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        let name = file.name().to_lowercase();
        if name.ends_with("amd64/wintun.dll") || name.ends_with("amd64\\wintun.dll") {
            // Directory already created above
            let dest = dest_dir.join("wintun.dll");
            let mut out = std::fs::File::create(&dest)?;
            let mut buf = Vec::new();
            use std::io::Read;
            file.read_to_end(&mut buf)?;
            use std::io::Write;
            out.write_all(&buf)?;
            return Ok(());
        }
    }

    Err(anyhow!("WinTun zip 中未找到 amd64/wintun.dll"))
}
