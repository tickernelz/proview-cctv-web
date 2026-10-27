use proview_core::cloud::dss::envelope_client;

const KEYS: [&str; 3] = ["PROVIEW_APP_UUID", "PROVIEW_APP_KEY", "PROVIEW_APP_SECRET"];

#[test]
fn identity_filled_after_the_first_call_is_used_by_the_next() {
    for key in KEYS {
        std::env::remove_var(key);
    }
    assert!(
        envelope_client().is_none(),
        "no identity in the environment must mean no client"
    );
    std::env::set_var("PROVIEW_APP_UUID", "uuid-from-settings");
    std::env::set_var("PROVIEW_APP_KEY", "key-from-settings");
    std::env::set_var("PROVIEW_APP_SECRET", "secret-from-settings");
    assert!(
        envelope_client().is_some(),
        "identity added after the first call must be picked up without a restart"
    );
    for key in KEYS {
        std::env::remove_var(key);
    }
}
