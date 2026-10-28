<script lang="ts">
  import { onMount } from 'svelte';
  import { type Device, loadDevices } from '#lib/api';

  let devices = $state<Device[]>([]);
  let selected = $state<string | null>(null);
  const online = $derived(devices.filter((d) => d.online).length);
  const title = $derived(devices.find((d) => d.id === selected)?.name ?? 'No device');

  async function refresh() {
    devices = await loadDevices();
  }

  onMount(refresh);
</script>

<main>
  <nav class="rail">
    <p>{online} of {devices.length} online</p>
    <button onclick={refresh}>Refresh</button>
    {#each devices as d (d.id)}
      <button aria-pressed={d.id === selected} onclick={() => (selected = d.id)}>{d.name}</button>
    {/each}
  </nav>
  <video muted {title}></video>
</main>
