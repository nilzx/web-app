//! Desktop shortcuts that start a single app directly (`webdock --app <id>`).
//! If WebDock is already running, the single-instance plugin forwards the
//! arguments to the running process instead.

use std::path::PathBuf;

#[cfg(windows)]
use anyhow::Context;
use anyhow::Result;
use tauri::AppHandle;
#[cfg(any(windows, target_os = "linux"))]
use tauri::Manager;

use crate::apps::WebApp;
use crate::shell::Shell;

/// The executable to put into shortcuts (the AppImage itself when running as one).
fn launcher_exe() -> Result<PathBuf> {
    if let Some(appimage) = std::env::var_os("APPIMAGE") {
        return Ok(PathBuf::from(appimage));
    }
    Ok(std::env::current_exe()?)
}

fn args(shell: &Shell, id: &str) -> Vec<String> {
    let mut args = vec!["--app".to_string(), id.to_string()];
    // Only pin the config file when it isn't found automatically.
    let default_lookup = crate::config::locate(None, crate::system_config_dir().as_deref());
    if default_lookup != shell.paths.config_file {
        args.push("--config".into());
        args.push(shell.paths.config_file.display().to_string());
    }
    args
}

fn quote(s: &str) -> String {
    if s.is_empty() || s.contains([' ', '"', '\'', '\\', '$', '`', '&', '(', ')']) {
        format!(
            "\"{}\"",
            s.replace('\\', "\\\\")
                .replace('"', "\\\"")
                .replace('$', "\\$")
                .replace('`', "\\`")
        )
    } else {
        s.to_string()
    }
}

pub fn command_line(shell: &Shell, id: &str) -> String {
    let exe = launcher_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "webdock".into());
    std::iter::once(exe)
        .chain(args(shell, id))
        .map(|a| quote(&a))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(target_os = "linux")]
pub fn create(app: &AppHandle, shell: &Shell, web_app: &WebApp) -> Result<PathBuf> {
    use std::os::unix::fs::PermissionsExt;

    let exec = command_line(shell, &web_app.id).replace('%', "%%");
    let icon = web_app
        .icon
        .as_ref()
        .filter(|p| p.extension().is_some_and(|e| e == "png" || e == "svg"))
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "applications-internet".into());
    let entry = format!(
        "[Desktop Entry]\nType=Application\nVersion=1.0\nName={}\nComment={}\nExec={}\nIcon={}\nTerminal=false\nCategories=Network;Utility;\nStartupWMClass=webdock\n",
        one_line(&web_app.name),
        one_line(&web_app.description),
        exec,
        icon
    );
    let file_name = format!("webdock-{}.desktop", web_app.id);

    // Application menu entry …
    let menu_dir = app.path().data_dir()?.join("applications");
    std::fs::create_dir_all(&menu_dir)?;
    std::fs::write(menu_dir.join(&file_name), &entry)?;

    // … and a desktop icon when a desktop folder exists.
    let target = match app.path().desktop_dir().ok().filter(|d| d.is_dir()) {
        Some(desktop) => desktop.join(&file_name),
        None => menu_dir.join(&file_name),
    };
    std::fs::write(&target, &entry)?;
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755))?;
    Ok(target)
}

#[cfg(target_os = "linux")]
fn one_line(s: &str) -> String {
    s.replace(['\n', '\r'], " ")
}

#[cfg(windows)]
pub fn create(app: &AppHandle, shell: &Shell, web_app: &WebApp) -> Result<PathBuf> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let desktop = app.path().desktop_dir().context("no desktop folder")?;
    let safe_name: String = web_app
        .name
        .chars()
        .map(|c| if "\\/:*?\"<>|".contains(c) { '_' } else { c })
        .collect();
    let lnk = desktop.join(format!("{}.lnk", safe_name.trim()));
    let exe = launcher_exe()?;
    let arguments = args(shell, &web_app.id)
        .iter()
        .map(|a| {
            if a.contains(' ') {
                format!("\"{a}\"")
            } else {
                a.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    let icon = web_app
        .icon
        .as_ref()
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("ico")))
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| format!("{},0", exe.display()));
    // PowerShell single-quoted strings: escape ' as ''.
    let q = |s: &str| format!("'{}'", s.replace('\'', "''"));
    let script = format!(
        "$s=(New-Object -ComObject WScript.Shell).CreateShortcut({});$s.TargetPath={};$s.Arguments={};$s.IconLocation={};$s.WorkingDirectory={};$s.Description={};$s.Save()",
        q(&lnk.display().to_string()),
        q(&exe.display().to_string()),
        q(&arguments),
        q(&icon),
        q(&exe
            .parent()
            .map(|p| p.display().to_string())
            .unwrap_or_default()),
        q(&web_app.description),
    );
    let status = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .creation_flags(CREATE_NO_WINDOW)
        .status()
        .context("run powershell")?;
    anyhow::ensure!(status.success(), "powershell exited with {status}");
    Ok(lnk)
}

#[cfg(not(any(windows, target_os = "linux")))]
pub fn create(_app: &AppHandle, _shell: &Shell, _web_app: &WebApp) -> Result<PathBuf> {
    anyhow::bail!(
        "desktop shortcuts are not supported on this platform; use the launch command instead"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting() {
        assert_eq!(quote("abc"), "abc");
        assert_eq!(quote("a b"), "\"a b\"");
        assert_eq!(quote("a\"b c"), "\"a\\\"b c\"");
    }
}
