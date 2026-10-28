import './app.css';
import { type Device, loadDevices } from './api';

const root = document.querySelector<HTMLDivElement>('#app')!;
let devices: Device[] = [];
let selected: string | null = null;

function render() {
  const online = devices.filter((d) => d.online).length;
  root.replaceChildren();
  const main = document.createElement('main');
  const rail = document.createElement('nav');
  rail.className = 'rail';
  const head = document.createElement('p');
  head.textContent = online + ' of ' + devices.length + ' online';
  const refresh = document.createElement('button');
  refresh.textContent = 'Refresh';
  refresh.onclick = () => void refreshDevices();
  rail.append(head, refresh);
  for (const d of devices) {
    const b = document.createElement('button');
    b.textContent = d.name;
    b.setAttribute('aria-pressed', String(d.id === selected));
    b.onclick = () => {
      selected = d.id;
      render();
    };
    rail.append(b);
  }
  const video = document.createElement('video');
  video.muted = true;
  video.title = devices.find((d) => d.id === selected)?.name ?? 'No device';
  main.append(rail, video);
  root.append(main);
}

async function refreshDevices() {
  devices = await loadDevices();
  render();
}

render();
void refreshDevices();
