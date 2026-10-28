use proview_core::cloud::client::CloudClient;

fn env(key: &str) -> String {
    std::env::var(key).unwrap_or_default().trim().to_string()
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = CloudClient::from_env();
    let captcha = client.captcha().await?;
    let payload = captcha
        .image
        .split_once(',')
        .map(|(_, rest)| rest.to_string())
        .unwrap_or_else(|| captcha.image.clone());
    std::fs::write("/tmp/xm-captcha.img", base64_decode(&payload))?;
    std::fs::write("/tmp/xm-captcha.token", &captcha.token)?;
    println!(
        "saved /tmp/xm-captcha.img bytes={}",
        std::fs::metadata("/tmp/xm-captcha.img")?.len()
    );
    Ok(())
}

fn base64_decode(text: &str) -> Vec<u8> {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = Vec::new();
    let mut buffer = 0u32;
    let mut bits = 0u8;
    for byte in text.bytes() {
        if byte == b'=' {
            break;
        }
        let value = match TABLE.iter().position(|item| *item == byte) {
            Some(index) => index as u32,
            None => continue,
        };
        buffer = (buffer << 6) | value;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
        }
    }
    out
}
