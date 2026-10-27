use std::fs;

use proview_core::cloud::{
    body_key, open, scramble, seal, signature, signed_request, url_encode, Identity,
};

fn fixture() -> serde_json::Value {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/cloud-envelope.json"
    );
    let text = fs::read_to_string(path).expect("fixture file");
    serde_json::from_str(&text).expect("fixture json")
}

#[test]
fn scramble_matches_the_captured_rows() {
    let fixture = fixture();
    for row in fixture["scramble"].as_array().expect("scramble rows") {
        let input = row["input"].as_str().expect("input");
        let expected = row["output"].as_str().expect("output");
        assert_eq!(&scramble(input, 4), expected, "scramble of {input:?}");
    }
}

#[test]
fn signature_and_key_match_the_captured_rows() {
    let fixture = fixture();
    for group in ["vectors", "uninitialised"] {
        for row in fixture[group].as_array().expect("rows") {
            let identity = Identity::new(
                row["uuid"].as_str().expect("uuid"),
                row["app_key"].as_str().expect("app key"),
                row["app_secret"].as_str().expect("app secret"),
            );
            let tsid = row["tsid"].as_str().expect("tsid");
            let millis: u64 = tsid[7..].parse().expect("millis");
            let counter: u32 = tsid[..7].parse().expect("counter");
            let signed = signed_request(&identity, counter, millis);
            assert_eq!(signed.tsid, tsid);
            assert_eq!(
                signed.sig,
                row["sig"].as_str().expect("sig"),
                "{group} {tsid}"
            );
            assert_eq!(
                signed.key,
                row["key"].as_str().expect("key"),
                "{group} {tsid}"
            );
            assert_eq!(body_key(tsid, &identity.app_secret), signed.key);
            let source = format!(
                "{}{}{}{}",
                identity.uuid, identity.app_key, identity.app_secret, tsid
            );
            assert_eq!(signature(&source, 4), signed.sig);
        }
    }
}

#[test]
fn sealed_body_matches_the_fixture_pair() {
    let fixture = fixture();
    let body = &fixture["body"];
    let key = body["key"].as_str().expect("key");
    let plain = body["plaintext"].as_str().expect("plaintext");
    let expected = body["ciphertext"].as_str().expect("ciphertext");
    assert_eq!(seal(key, plain).expect("seal"), expected);
    assert_eq!(open(key, expected).expect("open"), plain);
}

#[test]
fn credentials_never_reach_the_signed_body_unencoded() {
    let fixture = fixture();
    let key = fixture["body"]["key"].as_str().expect("key");
    let body = format!(
        "account=demo@example.com&pass={}",
        url_encode("p@ss word&x")
    );
    let sealed = seal(key, &body).expect("seal");
    assert_eq!(open(key, &sealed).expect("open"), body);
    assert!(body.ends_with("pass=p%40ss%20word%26x"));
}
