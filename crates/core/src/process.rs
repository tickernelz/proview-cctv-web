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
}
