use proview_core::cloud::Identity;

#[test]
fn the_environment_wins_where_it_is_filled() {
    let runtime = Identity::new("env-uuid", "", "env-secret");
    let built = Identity::new("built-uuid", "built-key", "built-secret");
    let identity = Identity::layered(runtime, built);
    assert_eq!(identity.uuid, "env-uuid");
    assert_eq!(identity.app_key, "built-key");
    assert_eq!(identity.app_secret, "env-secret");
}

#[test]
fn the_built_identity_fills_a_clean_environment() {
    let identity = Identity::layered(
        Identity::default(),
        Identity::new("built-uuid", "built-key", "built-secret"),
    );
    assert!(identity.is_complete());
    assert_eq!(identity.uuid, "built-uuid");
    assert_eq!(identity.app_key, "built-key");
    assert_eq!(identity.app_secret, "built-secret");
}

#[test]
fn blank_environment_values_do_not_beat_the_built_identity() {
    let identity = Identity::layered(
        Identity::new("  ", "key", "\t"),
        Identity::new("built-uuid", "built-key", "built-secret"),
    );
    assert_eq!(identity.uuid, "built-uuid");
    assert_eq!(identity.app_key, "key");
    assert_eq!(identity.app_secret, "built-secret");
}

#[test]
fn no_source_leaves_the_identity_incomplete() {
    let identity = Identity::layered(Identity::default(), Identity::default());
    assert!(!identity.is_complete());
}
