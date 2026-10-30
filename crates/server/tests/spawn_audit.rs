use std::path::{Path, PathBuf};

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root")
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

fn spawns_without_hidden_console(source: &str) -> Vec<usize> {
    let lines: Vec<&str> = source.lines().collect();
    let mut missing = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        let mut from = 0;
        while let Some(found) = line[from..].find("Command::new(") {
            let at = from + found;
            let before = line[..at].chars().last();
            let process = !before.is_some_and(|letter| letter.is_alphanumeric() || letter == '_');
            if process {
                let window = lines[index..(index + 4).min(lines.len())].join("\n");
                if !window.contains("hide_console(") {
                    missing.push(index + 1);
                }
            }
            from = at + 1;
        }
    }
    missing
}

#[test]
fn every_shipped_child_process_hides_its_console_window() {
    let root = workspace();
    let mut files = Vec::new();
    for dir in ["crates/core/src", "crates/server/src", "src-tauri/src"] {
        rust_files(&root.join(dir), &mut files);
    }
    assert!(
        files.len() > 20,
        "the audit found too few sources: {}",
        files.len()
    );
    let mut spawns = 0;
    let mut offenders = Vec::new();
    for file in &files {
        let source = std::fs::read_to_string(file).expect("source");
        spawns += source.matches("Command::new(").count();
        for line in spawns_without_hidden_console(&source) {
            offenders.push(format!("{}:{line}", file.display()));
        }
    }
    assert!(spawns >= 2, "the audit no longer sees the ffmpeg spawns");
    assert!(
        offenders.is_empty(),
        "spawns that open a console window on Windows: {offenders:?}"
    );
}

#[test]
fn the_audit_flags_a_spawn_without_the_flag_and_ignores_other_constructors() {
    let bare = "let mut command = Command::new(\"ffmpeg\");\ncommand.arg(\"-i\");\n";
    assert_eq!(spawns_without_hidden_console(bare), vec![1]);
    let hidden = "let mut command = tokio::process::Command::new(binary);\nproview_core::process::hide_console(command.as_std_mut());\n";
    assert!(spawns_without_hidden_console(hidden).is_empty());
    let other = "let command = PtzCommand::new(\"DirectionUp\");\n";
    assert!(spawns_without_hidden_console(other).is_empty());
}
