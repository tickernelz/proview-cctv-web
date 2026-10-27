use std::env;
use std::fs;
use std::path::Path;

const IDENTITY_FILE: &str = "../../cloud-identity.env";

const PAIRS: [(&str, &str); 3] = [
    ("PROVIEW_APP_UUID", "PROVIEW_BUILT_APP_UUID"),
    ("PROVIEW_APP_KEY", "PROVIEW_BUILT_APP_KEY"),
    ("PROVIEW_APP_SECRET", "PROVIEW_BUILT_APP_SECRET"),
];

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=PROVIEW_REQUIRE_CLOUD_IDENTITY");
    for (source, _) in PAIRS {
        println!("cargo:rerun-if-env-changed={source}");
    }
    let present = Path::new(IDENTITY_FILE).exists();
    if present {
        println!("cargo:rerun-if-changed={IDENTITY_FILE}");
    }

    let file = fs::read_to_string(IDENTITY_FILE).unwrap_or_default();
    let required = flag("PROVIEW_REQUIRE_CLOUD_IDENTITY") || present;
    let resolved: Vec<(&str, &str, Option<String>)> = PAIRS
        .iter()
        .map(|(source, target)| (*source, *target, value(source, &file)))
        .collect();
    let missing: Vec<&str> = resolved
        .iter()
        .filter(|(_, _, found)| found.is_none())
        .map(|(source, _, _)| *source)
        .collect();
    if !missing.is_empty() {
        if required {
            eprintln!(
                "proview: this build must carry a cloud identity; missing {}",
                missing.join(", ")
            );
            std::process::exit(1);
        }
        return;
    }
    for (_, target, found) in resolved {
        if let Some(found) = found {
            println!("cargo:rustc-env={target}={found}");
        }
    }
    println!(
        "cargo:warning=proview: cloud identity baked into this build (uuid, key, secret present; values not printed)"
    );
}

fn flag(key: &str) -> bool {
    matches!(
        env::var(key)
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
            .as_str(),
        "1" | "true" | "yes"
    )
}

fn value(key: &str, file: &str) -> Option<String> {
    env::var(key).ok().and_then(clean).or_else(|| {
        file.lines()
            .filter_map(|line| line.split_once('='))
            .find(|(name, _)| name.trim() == key)
            .and_then(|(_, value)| clean(value.to_string()))
    })
}

fn clean(value: String) -> Option<String> {
    let value = value.trim().to_string();
    if value.is_empty() || value.chars().any(char::is_control) {
        return None;
    }
    Some(value)
}
