use dioxus::prelude::*;

use serde::Deserialize;

#[derive(Clone, PartialEq, Deserialize)]
struct Device {
    id: String,
    name: String,
    online: bool,
}

async fn load_devices() -> Vec<Device> {
    match gloo_net::http::Request::get("/api/devices").send().await {
        Ok(res) if res.ok() => res.json().await.unwrap_or_default(),
        _ => Vec::new(),
    }
}

fn title_of(devices: &[Device], selected: Option<&str>) -> String {
    devices
        .iter()
        .find(|d| Some(d.id.as_str()) == selected)
        .map(|d| d.name.clone())
        .unwrap_or_else(|| "No device".into())
}

#[component]
fn App() -> Element {
    let devices = use_signal(Vec::<Device>::new);
    let mut selected = use_signal(|| None::<String>);
    let refresh = move || {
        spawn(async move {
            let mut devices = devices;
            devices.set(load_devices().await);
        });
    };
    use_hook(refresh);
    let online = devices.read().iter().filter(|d| d.online).count();
    let total = devices.read().len();
    let title = title_of(&devices.read(), selected.read().as_deref());
    rsx! {
        main {
            nav { class: "rail",
                p { "{online} of {total} online" }
                button { onclick: move |_| refresh(), "Refresh" }
                for d in devices.read().iter().cloned() {
                    button {
                        key: "{d.id}",
                        "aria-pressed": "{selected.read().as_deref() == Some(d.id.as_str())}",
                        onclick: move |_| selected.set(Some(d.id.clone())),
                        "{d.name}"
                    }
                }
            }
            video { muted: true, title: "{title}" }
        }
    }
}

fn main() {
    dioxus::launch(App);
}
