use proview_core::rtsp;

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let host = args.next().unwrap_or_else(|| "127.0.0.1".to_string());
    let port: u16 = args.next().and_then(|p| p.parse().ok()).unwrap_or(554);
    let user = std::env::var("PROVIEW_DEVICE_USER").unwrap_or_else(|_| "admin".to_string());
    let pass = std::env::var("PROVIEW_DEVICE_PASS").unwrap_or_default();

    match rtsp::describe(&host, port, "/", &user, &pass).await {
        Ok(info) => println!(
            "ok video={:?} audio={:?} fps={:?}",
            info.video_codec, info.audio_codec, info.framerate
        ),
        Err(error) => println!("error: {error}"),
    }
}
