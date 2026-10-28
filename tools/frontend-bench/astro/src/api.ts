export type Device = { id: string; name: string; online: boolean };
export async function loadDevices(): Promise<Device[]> {
  const res = await fetch('/api/devices');
  if (!res.ok) return [];
  return (await res.json()) as Device[];
}
