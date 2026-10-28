use yew::prelude::*;

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

#[function_component]
fn App() -> Html {
    let devices = use_state(Vec::<Device>::new);
    let selected = use_state(|| None::<String>);
    let refresh = {
        let devices = devices.clone();
        Callback::from(move |_: ()| {
            let devices = devices.clone();
            wasm_bindgen_futures::spawn_local(async move { devices.set(load_devices().await) });
        })
    };
    {
        let refresh = refresh.clone();
        use_effect_with((), move |_| refresh.emit(()));
    }
    let online = devices.iter().filter(|d| d.online).count();
    let title = title_of(&devices, selected.as_deref());
    html! {
        <main>
            <nav class="rail">
                <p>{ format!("{online} of {} online", devices.len()) }</p>
                <button onclick={ move |_| refresh.emit(()) }>{ "Refresh" }</button>
                { for devices.iter().map(|d| {
                    let pick = d.id.clone();
                    let selected_handle = selected.clone();
                    html! {
                        <button
                            key={ d.id.clone() }
                            aria-pressed={ (selected.as_deref() == Some(d.id.as_str())).to_string() }
                            onclick={ move |_| selected_handle.set(Some(pick.clone())) }
                        >{ d.name.clone() }</button>
                    }
                }) }
            </nav>
            <video muted=true title={ title }></video>
        </main>
    }
}

fn main() {
    yew::Renderer::<App>::new().render();
}
