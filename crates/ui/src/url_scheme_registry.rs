use std::process::Command;

/// Register the `dbstudio://` URL scheme for the current user.
///
/// On Windows this writes `HKCU\Software\Classes\dbstudio` registry entries
/// pointing at the currently running exe. On macOS/Linux the scheme can be
/// registered with OS-specific tools, which are invoked if available.
pub fn register_url_scheme() -> anyhow::Result<()> {
    match std::env::consts::OS {
        "windows" => register_windows(),
        "macos" => register_macos(),
        "linux" => register_linux(),
        _ => Ok(()),
    }
}

#[cfg(windows)]
fn register_windows() -> anyhow::Result<()> {
    let exe = std::env::current_exe()?;
    let exe_quoted = format!("\"{}\" \"%1\"", exe.display());
    let exe_path_quoted = format!("\"{}\"", exe.display());

    // HKCU\Software\Classes\dbstudio
    run_reg(&["add", "HKCU\\Software\\Classes\\dbstudio", "/ve", "/d", "URL:dbstudio Protocol", "/f"])?;
    run_reg(&["add", "HKCU\\Software\\Classes\\dbstudio", "/v", "URL Protocol", "/d", "", "/f"])?;
    // DefaultIcon
    run_reg(&[
        "add", "HKCU\\Software\\Classes\\dbstudio\\DefaultIcon",
        "/ve", "/d", exe_path_quoted.as_str(), "/f",
    ])?;
    // shell\open\command
    run_reg(&[
        "add", "HKCU\\Software\\Classes\\dbstudio\\shell\\open\\command",
        "/ve", "/d", exe_quoted.as_str(), "/f",
    ])?;
    Ok(())
}

#[cfg(not(windows))]
fn register_windows() -> anyhow::Result<()> {
    Ok(())
}

fn register_macos() -> anyhow::Result<()> {
    // Plist-based registration is app-bundle dependent; no-op is acceptable.
    tracing::warn!("dbstudio:// macOS registration not implemented (requires Info.plist CFBundleURLTypes)");
    Ok(())
}

fn register_linux() -> anyhow::Result<()> {
    let desktop_id = "dbstudio.desktop";
    let desktop_path = format!("{}/.local/share/applications/{}", std::env::var("HOME")? , desktop_id);
    let content = format!(
        "[Desktop Entry]\nType=Application\nName=dbstudio\nExec={} %u\nMimeType=x-scheme-handler/dbstudio;\n",
        std::env::current_exe()?.display()
    );
    std::fs::create_dir_all(std::path::Path::new(&desktop_path).parent().unwrap())?;
    std::fs::write(&desktop_path, content)?;
    let _ = Command::new("xdg-mime").args(["default", desktop_id, "x-scheme-handler/dbstudio"]).output();
    Ok(())
}

#[cfg(windows)]
fn run_reg(args: &[&str]) -> anyhow::Result<()> {
    let output = Command::new("reg.exe").args(args).output()?;
    if !output.status.success() {
        anyhow::bail!("reg.exe {:?} failed: {}", &args[1..], String::from_utf8_lossy(&output.stderr));
    }
    Ok(())
}