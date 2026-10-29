use std::process::Command;

pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub fn hide_console(command: &mut Command) -> u32 {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
        CREATE_NO_WINDOW
    }
    #[cfg(not(windows))]
    {
        let _ = command;
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hides_the_console_window_on_windows_only() {
        let mut command = Command::new("ffmpeg");
        command.args(["-hide_banner", "pipe:1"]);
        let applied = hide_console(&mut command);
        #[cfg(windows)]
        assert_eq!(applied, CREATE_NO_WINDOW);
        #[cfg(not(windows))]
        assert_eq!(applied, 0);
        let args: Vec<String> = command
            .get_args()
            .map(|arg| arg.to_string_lossy().to_string())
            .collect();
        assert_eq!(args, ["-hide_banner", "pipe:1"]);
    }

    #[cfg(windows)]
    #[test]
    fn a_hidden_child_reports_no_console_window() {
        use std::os::windows::process::CommandExt;

        const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;

        fn console_width(flags: u32) -> Option<Option<i32>> {
            let mut command = Command::new("powershell");
            command.args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "[Console]::WindowWidth",
            ]);
            command
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null())
                .creation_flags(flags);
            let output = command.output().ok()?;
            if !output.status.success() {
                return Some(None);
            }
            let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
            Some(text.parse::<i32>().ok())
        }

        let control = console_width(CREATE_NEW_CONSOLE).expect("the control child must start");
        assert!(
            matches!(control, Some(width) if width > 0),
            "a child started with CREATE_NEW_CONSOLE must own a console: {control:?}"
        );
        assert_eq!(
            console_width(CREATE_NO_WINDOW),
            Some(None),
            "a child started with CREATE_NO_WINDOW must not own a console"
        );
    }
}
