//! F10: a terminal opened in the current folder, as in OneCommander and
//! Explorer's «Open in Terminal». Windows Terminal when installed, else
//! PowerShell.

use std::path::Path;

/// The program and its raw arguments that open a terminal in `folder`.
pub fn command(folder: &str, windows_terminal: bool) -> (String, String) {
    // `wt -d "C:\"` would read the closing quote as escaped: "C:\." is the
    // same folder without that trap.
    let folder = if folder.ends_with('\\') { format!("{folder}.") } else { folder.to_owned() };
    if windows_terminal {
        ("wt.exe".into(), format!("-d \"{folder}\""))
    } else {
        // PowerShell, unlike cmd, can stand in a \\server\share folder.
        ("powershell.exe".into(), format!("-NoExit -Command Set-Location -LiteralPath '{}'", folder.replace('\'', "''")))
    }
}

/// Opens the terminal; `admin` asks Windows for elevation first.
#[cfg(windows)]
pub fn open(folder: &Path, admin: bool) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::process::CommandExt;
    let folder = crate::path_utils::normalize_extended_path(folder);
    let windows_terminal = std::env::var_os("LOCALAPPDATA")
        .is_some_and(|dir| Path::new(&dir).join(r"Microsoft\WindowsApps\wt.exe").exists());
    let (program, arguments) = command(&folder.to_string_lossy(), windows_terminal);
    if !admin {
        const CREATE_NEW_CONSOLE: u32 = 0x10;
        std::process::Command::new(&program).raw_arg(&arguments).current_dir(&folder)
            .creation_flags(CREATE_NEW_CONSOLE).spawn()?;
        return Ok(());
    }
    let wide = |text: &std::ffi::OsStr| text.encode_wide().chain([0]).collect::<Vec<u16>>();
    let (verb, file, params, dir) = (wide("runas".as_ref()), wide(program.as_ref()), wide(arguments.as_ref()), wide(folder.as_os_str()));
    let result = unsafe {
        windows_sys::Win32::UI::Shell::ShellExecuteW(std::ptr::null_mut(), verb.as_ptr(), file.as_ptr(), params.as_ptr(), dir.as_ptr(),
            windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL)
    };
    // Above 32 means started; the user declining elevation is an error too.
    if result as isize > 32 { Ok(()) } else { Err(std::io::Error::other("Windows не запустил терминал (повышение прав отклонено?)")) }
}

#[cfg(not(windows))]
pub fn open(_folder: &Path, _admin: bool) -> std::io::Result<()> {
    Err(std::io::Error::other("Терминал открывается только в Windows"))
}

#[cfg(test)]
mod tests {
    use super::command;

    #[test]
    fn windows_terminal_and_powershell_lines() {
        assert_eq!(command(r"D:\Проекты", true), ("wt.exe".into(), r#"-d "D:\Проекты""#.into()));
        assert_eq!(command(r"C:\", true), ("wt.exe".into(), r#"-d "C:\.""#.into()));
        assert_eq!(command(r"\\srv\share\O'Neil", false),
            ("powershell.exe".into(), r"-NoExit -Command Set-Location -LiteralPath '\\srv\share\O''Neil'".into()));
    }
}
