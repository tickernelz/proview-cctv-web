use proview_core::cloud::envelope::{
    now_millis, open, seal, signed_request, url_encode, EnvelopeClient, Identity,
};

async fn call(
    http: &reqwest::Client,
    identity: &Identity,
    base: &str,
    endpoint: &str,
    counter: u32,
    body: Option<&str>,
    access: Option<&str>,
) -> Result<String, Box<dyn std::error::Error>> {
    let signed = signed_request(identity, counter, now_millis());
    let url = format!(
        "{}/{}/{}/{}/{}.rs",
        base, endpoint, "v3", signed.tsid, signed.sig
    );
    let mut request = http
        .post(url)
        .header(
            "Accept",
            "text/html, image/gif, image/jpeg, *; q=.2, */*; q=.2",
        )
        .header("Accept-Charset", "utf-8")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .header("uuid", &identity.uuid)
        .header("appKey", &identity.app_key);
    if let Some(token) = access {
        request = request.header("Authorization", token);
    }
    if let Some(plain) = body {
        request = request.body(seal(&signed.key, plain)?);
    }
    let response = request.send().await?;
    Ok(open(&signed.key, &response.text().await?)?)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let account = std::env::var("PROVIEW_EMAIL")?.trim().to_string();
    let password = std::env::var("PROVIEW_PASSWORD")?.trim().to_string();
    let identity = Identity::resolve();
    let client = EnvelopeClient::new(identity.clone());
    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()?;
    let body = format!("account={}&pass={}", account, url_encode(&password));
    let login: serde_json::Value = serde_json::from_str(
        &call(
            &http,
            &identity,
            client.base(),
            "loginExist",
            1,
            Some(&body),
            None,
        )
        .await?,
    )?;
    let access = login["data"]["accessToken"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    let list: serde_json::Value = serde_json::from_str(
        &call(
            &http,
            &identity,
            client.base(),
            "msdlistCode",
            2,
            None,
            Some(&access),
        )
        .await?,
    )?;
    let out = std::env::var("PROBE_OUT")
        .unwrap_or_else(|_| "/home/zhafron/re/proview-cctv/captures/devdata.json".to_string());
    std::fs::write(&out, serde_json::to_string_pretty(&list["data"])?)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&out, std::fs::Permissions::from_mode(0o600));
    }
    println!(
        "saved {} mine={} share={} userId_len={}",
        out,
        list["data"]["mine"]
            .as_array()
            .map(|a| a.len())
            .unwrap_or(0),
        list["data"]["share"]
            .as_array()
            .map(|a| a.len())
            .unwrap_or(0),
        list["data"]["userId"]
            .as_str()
            .map(|s| s.len())
            .unwrap_or(0)
    );
    Ok(())
}
