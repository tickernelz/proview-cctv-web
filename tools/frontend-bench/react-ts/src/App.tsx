import { useEffect, useState } from 'react';
import { type Device, loadDevices } from './api';

export default function App() {
  const [devices, setDevices] = useState<Device[]>([]);
  const [selected, setSelected] = useState<string | null>(null);
  const online = devices.filter((d) => d.online).length;
  const title = devices.find((d) => d.id === selected)?.name ?? 'No device';
  const refresh = () => void loadDevices().then(setDevices);
  useEffect(refresh, []);
  return (
    <main>
      <nav className="rail">
        <p>{online} of {devices.length} online</p>
        <button onClick={refresh}>Refresh</button>
        {devices.map((d) => (
          <button key={d.id} aria-pressed={d.id === selected} onClick={() => setSelected(d.id)}>{d.name}</button>
        ))}
      </nav>
      <video muted title={title}></video>
    </main>
  );
}
