use std::fs;

use proview_core::cloud::{
    body_key, open, scramble, seal, signature, signed_request, url_encode, CloudError,
    EnvelopeClient, Identity,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

fn fixture() -> serde_json::Value {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/cloud-envelope.json"
    );
    let text = fs::read_to_string(path).expect("fixture file");
    serde_json::from_str(&text).expect("fixture json")
}

#[test]
fn scramble_matches_the_reference_vectors() {
    let fixture = fixture();
    for row in fixture["scramble"].as_array().expect("scramble rows") {
        let input = row["input"].as_str().expect("input");
        let expected = row["output"].as_str().expect("output");
        assert_eq!(&scramble(input, 4), expected, "scramble of {input:?}");
    }
}

#[test]
fn signature_and_key_match_the_reference_vectors() {
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

async fn stub_vendor(body: &'static str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("stub listener");
    let address = listener.local_addr().expect("stub address");
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            tokio::spawn(async move {
                let mut buffer = vec![0u8; 4096];
                let _ = socket.read(&mut buffer).await;
                let reply = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = socket.write_all(reply.as_bytes()).await;
                let _ = socket.shutdown().await;
            });
        }
    });
    format!("http://{address}")
}

fn probe_client(base: String) -> EnvelopeClient {
    EnvelopeClient::with_base(
        Identity::new("probe-uuid", "probe-key", "probe-secret"),
        base,
    )
}

#[tokio::test]
async fn a_revoked_token_is_expired_on_the_device_list() {
    let base = stub_vendor(r#"{"code":-4004,"msg":"token revoked"}"#).await;
    let error = probe_client(base)
        .device_list("revoked-token")
        .await
        .expect_err("a revoked token cannot list devices");
    assert!(
        matches!(error, CloudError::Expired),
        "a revoked token is a session that ended, not a plain rejection: {error}"
    );
    assert_eq!(error.to_string(), "session expired; sign in again");
}

#[tokio::test]
async fn another_refusal_stays_a_rejection() {
    let base = stub_vendor(r#"{"code":-1001,"msg":"device list refused"}"#).await;
    let error = probe_client(base)
        .device_list("token")
        .await
        .expect_err("a refusal is not a device list");
    assert!(
        matches!(error, CloudError::Rejected(_)),
        "only the expired code means signed out: {error}"
    );
    assert!(error.to_string().contains("device list refused"), "{error}");
}
