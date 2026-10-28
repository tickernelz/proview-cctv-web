import { addLauncher, menuOpen } from '../live/store';

(function deviceAdmin() {
  'use strict';

  const API = {
    attributes: '/api/cloud/device-attributes',
    rename: '/api/cloud/device/rename',
    group: '/api/cloud/device/group',
    favourite: '/api/cloud/device/favourite',
    share: '/api/cloud/device/share',
    pending: '/api/cloud/device/share/pending',
    accept: '/api/cloud/device/share/accept',
    cancel: '/api/cloud/device/share/cancel',
    cloudPassword: '/api/cloud/device/password',
  };

  async function call(path, options = {}) {
    const response = await fetch(path, {
      credentials: 'same-origin',
      headers: { 'content-type': 'application/json' },
      ...options,
    });
    const text = await response.text();
    let body = null;
    if (text) {
      try {
        body = JSON.parse(text);
      } catch {
        body = null;
      }
    }
    if (!response.ok) {
      const message = body && body.message ? body.message : response.status + ' ' + response.statusText;
      const error = new Error(message);
      error.status = response.status;
      throw error;
    }
    return body;
  }

  function node(tag, props = {}, children = []) {
    const element = document.createElement(tag);
    for (const [key, value] of Object.entries(props)) {
      if (key === 'class') element.className = value;
      else if (key === 'text') element.textContent = value;
      else if (key.startsWith('on')) element.addEventListener(key.slice(2), value);
      else if (value !== null && value !== undefined) element.setAttribute(key, value);
    }
    for (const child of [].concat(children)) {
      if (child) element.append(child);
    }
    return element;
  }

  const state = { devices: [], groups: [], selected: '', busy: false };

  const status = node('p', { class: 'hint', role: 'status', id: 'device-admin-status', text: '' });
  const list = node('div', { class: 'rows', id: 'device-admin-list' });
  const groupsLine = node('p', { class: 'hint', id: 'device-admin-groups', text: '' });

  const renameInput = node('input', { type: 'text', id: 'device-admin-name', placeholder: 'Pintu keluar masuk' });
  const groupInput = node('input', { type: 'text', id: 'device-admin-group', placeholder: 'pintu, garasi, teras' });
  const favouriteBox = node('input', { type: 'checkbox', id: 'device-admin-favourite' });
  const shareInput = node('input', { type: 'text', id: 'device-admin-accept', placeholder: 'the other account email or its account id' });
  const shareKey = node('input', { type: 'text', id: 'device-admin-key', placeholder: 'transKey from the invitation' });
  const shareDevice = node('input', { type: 'text', id: 'device-admin-share-device', placeholder: 'camera serial shared with you' });
  const shareAccountPass = node('input', { type: 'password', id: 'device-admin-share-pass', placeholder: 'your own account password', autocomplete: 'current-password' });
  const cloudOld = node('input', { type: 'password', id: 'device-admin-old', placeholder: 'current device password', autocomplete: 'off' });
  const cloudNew = node('input', { type: 'password', id: 'device-admin-new', placeholder: 'new device password', autocomplete: 'new-password' });

  function field(label, control, hint) {
    return node('label', { class: 'field' }, [
      node('span', { class: 'lab', text: label }),
      control,
      hint ? node('span', { class: 'hint', text: hint }) : null,
    ]);
  }

  function say(message, kind) {
    status.textContent = message;
    status.dataset.kind = kind || '';
  }

  function selected() {
    return state.devices.find((device) => device.uuid === state.selected) || state.devices[0] || null;
  }

  async function guard(label, work) {
    if (state.busy) return;
    state.busy = true;
    say(label + '…');
    try {
      const result = await work();
      await refresh();
      say(result || 'done', 'ok');
    } catch (error) {
      say(error.message, 'bad');
    } finally {
      state.busy = false;
    }
  }

  function row(device) {
    const isSelected = device.uuid === state.selected;
    const tags = [];
    if (device.favourite) tags.push('favourite');
    if (device.group) tags.push(device.group);
    if (device.administrator) tags.push('administrator');
    if (device.shared_accounts) tags.push(device.shared_accounts + ' shared');
    return node('button', {
      type: 'button',
      class: 'menu-row' + (isSelected ? ' is-selected' : ''),
      role: 'menuitemradio',
      'aria-checked': isSelected ? 'true' : 'false',
      onclick: () => {
        state.selected = device.uuid;
        fill();
        render();
      },
    }, [
      node('span', { class: 'col' }, [
        node('span', { class: 'name', text: device.nickname }),
        node('span', { class: 'sub', text: device.uuid.slice(-6) + (tags.length ? ' · ' + tags.join(' · ') : '') }),
      ]),
    ]);
  }

  function fill() {
    const device = selected();
    if (!device) return;
    renameInput.value = device.nickname;
    groupInput.value = device.group || '';
    favouriteBox.checked = Boolean(device.favourite);
  }

  function render() {
    list.replaceChildren(...(state.devices.length ? state.devices.map(row) : [node('p', { class: 'hint', text: 'No camera on this account yet.' })]));
    groupsLine.textContent = state.groups.length ? 'Groups: ' + state.groups.join(', ') : 'No group yet.';
    fill();
  }

  async function refresh() {
    try {
      const payload = await call(API.attributes);
      state.devices = payload.devices || [];
      state.groups = payload.groups || [];
      if (!state.devices.some((device) => device.uuid === state.selected)) {
        state.selected = state.devices.length ? state.devices[0].uuid : '';
      }
      render();
    } catch (error) {
      list.replaceChildren(node('p', { class: 'hint', text: error.message }));
    }
  }

  function currentUuid() {
    const device = selected();
    return device ? device.uuid : '';
  }

  const sheet = node('section', { class: 'menu sheet', id: 'device-admin', role: 'dialog', 'aria-labelledby': 'device-admin-title', hidden: 'hidden' }, [
    node('div', { class: 'sheet-h' }, [
      node('h2', { id: 'device-admin-title', text: 'Devices on this account' }),
      node('button', {
        type: 'button', class: 'iconbtn', 'aria-label': 'Close', onclick: () => close(),
      }),
    ]),
    list,
    groupsLine,
    node('div', { class: 'rows' }, [
      field('Name', renameInput),
      node('button', {
        type: 'button', class: 'btn btn-fill', text: 'Rename through the cloud',
        onclick: () => guard('renaming', async () => {
          const uuid = currentUuid();
          if (!uuid) throw new Error('pick a camera first');
          const nickname = renameInput.value.trim();
          if (!nickname) throw new Error('the name is empty');
          await call(API.rename, { method: 'POST', body: JSON.stringify({ uuid, nickname }) });
          return 'renamed to ' + nickname;
        }),
      }),
    ]),
    node('div', { class: 'rows' }, [
      field('Group', groupInput, 'stored here, the cloud account carries no group field'),
      field('Favourite', favouriteBox),
      node('div', { class: 'rows' }, [
        node('button', {
          type: 'button', class: 'btn btn-text', text: 'Save group and favourite',
          onclick: () => guard('saving', async () => {
            const uuid = currentUuid();
            if (!uuid) throw new Error('pick a camera first');
            const groupReply = await call(API.group, { method: 'POST', body: JSON.stringify({ uuid, group: groupInput.value.trim() }) });
            const favouriteReply = await call(API.favourite, { method: 'POST', body: JSON.stringify({ uuid, favourite: favouriteBox.checked }) });
            return (favouriteReply && favouriteReply.message) || (groupReply && groupReply.message) || 'saved locally';
          }),
        }),
      ]),
    ]),
    node('div', { class: 'rows' }, [
      field('Share to account', shareInput, 'the receiving account email resolves to its account id first'),
      node('button', {
        type: 'button', class: 'btn btn-fill', text: 'Send the invitation',
        onclick: () => guard('sharing', async () => {
          const uuid = currentUuid();
          if (!uuid) throw new Error('pick a camera first');
          const target = shareInput.value.trim();
          if (!target) throw new Error('the receiving account is empty');
          const body = target.includes('@') ? { uuids: [uuid], account: target } : { uuids: [uuid], acceptId: target };
          const reply = await call(API.share, { method: 'POST', body: JSON.stringify(body) });
          const data = reply && reply.data;
          const key = data && (data.transKey || data.key || (data.transInfos && data.transInfos[0] && data.transInfos[0].transKey));
          if (key) shareKey.value = key;
          const invited = reply && reply.resolved ? ' to ' + reply.resolved : '';
          return key ? 'invitation ' + key + invited : 'the cloud accepted the invitation' + invited;
        }),
      }),
      field('Invitation key', shareKey),
      field('Camera shared with you', shareDevice, 'the receiver accepts a shared camera by its serial and its own account password'),
      field('Your account password', shareAccountPass, 'sent once inside the sealed body; the share service checks it'),
      node('div', { class: 'rows' }, [
        node('button', {
          type: 'button', class: 'btn btn-text', text: 'Accept',
          onclick: () => guard('accepting', async () => {
            const device = shareDevice.value.trim();
            const key = shareKey.value.trim();
            if (device) {
              const password = shareAccountPass.value;
              if (!password) throw new Error('the receiver needs its own account password');
              const reply = await call(API.accept, { method: 'POST', body: JSON.stringify({ devId: device, password }) });
              if (!reply || reply.materialised !== true) throw new Error('the camera did not land in this account');
              shareAccountPass.value = '';
              return 'accepted ' + device + ', it is in this account\'s device list';
            }
            if (!key) throw new Error('paste an invitation key first');
            await call(API.accept, { method: 'POST', body: JSON.stringify({ key }) });
            return 'transfer ' + key + ' acknowledged; the camera lands after Accept with its serial and your password';
          }),
        }),
        node('button', {
          type: 'button', class: 'btn btn-text', text: 'Check this camera',
          onclick: () => guard('checking', async () => {
            const uuid = currentUuid();
            if (!uuid) throw new Error('pick a camera first');
            const reply = await call(API.pending + '?uuids=' + encodeURIComponent(uuid));
            const rows = (reply && reply.data) || [];
            const pending = rows.filter((entry) => entry && entry.trans);
            if (!pending.length) return 'no pending invitation';
            shareKey.value = pending[0].transKey || shareKey.value;
            return pending.length + ' pending invitation';
          }),
        }),
        node('button', {
          type: 'button', class: 'btn btn-text', text: 'Cancel invitation',
          onclick: () => guard('cancelling', async () => {
            const key = shareKey.value.trim();
            const uuid = currentUuid();
            if (!key || !uuid) throw new Error('an invitation key and a camera are required');
            await call(API.cancel, { method: 'POST', body: JSON.stringify({ key, uuids: [uuid] }) });
            return 'cancelled ' + key;
          }),
        }),
      ]),
    ]),
    node('div', { class: 'rows' }, [
      node('p', { class: 'hint', text: 'Device password goes through the account: the camera picks it up on its next cloud call, and the server stores the new one for its own camera routes.' }),
      field('Current', cloudOld),
      field('New', cloudNew),
      node('button', {
        type: 'button', class: 'btn btn-fill', text: 'Change through the cloud',
        onclick: () => guard('changing', async () => {
          const uuid = currentUuid();
          if (!uuid) throw new Error('pick a camera first');
          const reply = await call(API.cloudPassword, {
            method: 'POST',
            body: JSON.stringify({ uuid, password: cloudOld.value, newPassword: cloudNew.value }),
          });
          cloudOld.value = '';
          cloudNew.value = '';
          return (reply && reply.message) || 'the cloud changed the device password';
        }),
      }),
    ]),
    status,
  ]);

  function open() {
    sheet.hidden = false;
    refresh();
  }

  function close() {
    sheet.hidden = true;
  }

  document.body.append(sheet);

  addLauncher({ id: 'device-admin-open', label: 'Devices', icon: 'cctv', slot: 'start', title: 'Device names, groups, sharing, passwords', onClick: open });
  sheet.querySelector('.sheet-h .iconbtn').innerHTML = '<svg class="i" aria-hidden="true"><use href="#i-x"></use></svg>';

  document.addEventListener('keydown', (event) => {
    if (event.key === 'Escape' && !event.defaultPrevented && !sheet.hidden && !menuOpen()) close();
  });

  window.proviewDeviceAdmin = { open, close, refresh };
})();
