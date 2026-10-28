use leptos::prelude::*;
use leptos::task::spawn_local;

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
fn App() -> impl IntoView {
    let devices = RwSignal::new(Vec::<Device>::new());
    let selected = RwSignal::new(None::<String>);
    let refresh = move || spawn_local(async move { devices.set(load_devices().await) });
    refresh();
    let online = move || devices.with(|d| d.iter().filter(|d| d.online).count());
    let total = move || devices.with(Vec::len);
    let title = move || devices.with(|d| title_of(d, selected.get().as_deref()));
    view! {
        <main>
            <nav class="rail">
                <p>{online} " of " {total} " online"</p>
                <button on:click=move |_| refresh()>"Refresh"</button>
                <For each=move || devices.get() key=|d| d.id.clone() let:d>
                    {
                        let id = d.id.clone();
                        let pick = d.id.clone();
                        view! {
                            <button
                                aria-pressed=move || (selected.get().as_deref() == Some(id.as_str())).to_string()
                                on:click=move |_| selected.set(Some(pick.clone()))
                            >
                                {d.name.clone()}
                            </button>
                        }
                    }
                </For>
            </nav>
            <video muted=true title=title></video>
        </main>
    }
}

fn main() {
    leptos::mount::mount_to_body(App)
}
