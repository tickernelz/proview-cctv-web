import { createMemo, createSignal, For, onMount } from 'solid-js';
import './app.css';
import { type Device, loadDevices } from './api';

export default function App() {
  const [devices, setDevices] = createSignal<Device[]>([]);
  const [selected, setSelected] = createSignal<string | null>(null);
  const online = createMemo(() => devices().filter((d) => d.online).length);
  const title = createMemo(() => devices().find((d) => d.id === selected())?.name ?? 'No device');
  const refresh = () => void loadDevices().then(setDevices);
  onMount(refresh);
  return (
    <main>
      <nav class="rail">
        <p>{online()} of {devices().length} online</p>
        <button onClick={refresh}>Refresh</button>
        <For each={devices()}>{(d) => <button aria-pressed={d.id === selected()} onClick={() => setSelected(d.id)}>{d.name}</button>}</For>
      </nav>
      <video muted title={title()}></video>
    </main>
  );
}
