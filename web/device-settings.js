const deviceSettings = (() => {
  const BASE = '/api/device/settings';
  const STREAMS = ['main', 'extra'];

  const node = (tag, props = {}, children = []) => {
    const element = document.createElement(tag);
    for (const [key, value] of Object.entries(props)) {
      if (key === 'text') {
        element.textContent = value;
      } else if (key === 'class') {
        element.className = value;
      } else if (key === 'dataset') {
        Object.assign(element.dataset, value);
      } else if (key.startsWith('on') && typeof value === 'function') {
        element.addEventListener(key.slice(2), value);
      } else if (value !== null && value !== undefined) {
        element.setAttribute(key, value);
      }
    }
    for (const child of children) {
      element.append(child);
    }
    return element;
  };

  const row = (label, hint, control) =>
    node('div', { class: 'row' }, [
      node('div', { class: 'lab', text: label }, hint ? [node('div', { class: 'hint', text: hint })] : []),
      node('div', { class: 'ctl-col' }, [control]),
    ]);

  const status = (text, bad = false) => {
    const line = document.getElementById('device-status');
    if (!line) {
      return;
    }
    line.textContent = text;
    line.classList.toggle('warn-text', bad);
  };

  const api = async (path, init) => {
    const response = await fetch(path, init);
    const raw = await response.text();
    let body = null;
    try {
      body = raw ? JSON.parse(raw) : null;
    } catch {
      body = { message: raw };
    }
    return { ok: response.ok, status: response.status, body };
  };

  const failureText = (result) =>
    (result.body && (result.body.message || result.body.error)) || 'HTTP ' + result.status;

  const number = (value) => {
    const input = node('input', { type: 'number', step: '1', min: '0' });
    input.value = value === null || value === undefined ? '' : String(value);
    return input;
  };

  const text = (value, placeholder = '') => {
    const input = node('input', { type: 'text', placeholder });
    input.value = value === null || value === undefined ? '' : String(value);
    return input;
  };

  const asNumber = (input) => (input.value === '' ? null : Number(input.value));

  const reportTable = (report) => {
    const fieldRow = (field, state) =>
      node('tr', {}, [
        node('th', { scope: 'row', text: field.field }),
        node('td', { class: 'code', text: JSON.stringify(field.before) }),
        node('td', { class: 'code', text: JSON.stringify(field.requested) }),
        node('td', { class: 'code', text: JSON.stringify(field.after) }),
        node('td', { text: state }),
      ]);
    const rows = report.fields.map((field) => fieldRow(field, field.matches ? 'matches' : 'differs'));
    const restored = report.restored.length
      ? [
          node('tr', {}, [node('th', { colspan: '5', scope: 'row', text: 'put back from the saved copy' })]),
          ...report.restored.map((field) => fieldRow(field, field.matches ? 'restored' : 'still differs')),
        ]
      : [];
    return node('div', { class: 'row' }, [
      node('div', { class: 'lab', text: report.block }),
      node('div', { class: 'ctl-col' }, [
        node('p', {
          class: 'hint',
          text:
            (report.reverted
              ? 'The device returned a different value, so the saved copy was put back. '
              : 'Read back and kept. ') + 'Copy: ' + report.backup,
        }),
        node('table', { class: 'tbl' }, [
          node('thead', {}, [
            node('tr', {}, [
              node('th', { scope: 'col', text: 'Field' }),
              node('th', { scope: 'col', text: 'Before' }),
              node('th', { scope: 'col', text: 'Asked for' }),
              node('th', { scope: 'col', text: 'After' }),
              node('th', { scope: 'col', text: 'State' }),
            ]),
          ]),
          node('tbody', {}, [...rows, ...restored]),
        ]),
      ]),
    ]);
  };

  const renderReports = (container, body) => {
    const reports = (body && body.reports) || [];
    const errors = (body && body.errors) || [];
    container.replaceChildren(...reports.map(reportTable));
    if (errors.length) {
      status(errors.join(' / '), true);
    } else if (body && body.reverted) {
      status('The write did not hold and was reverted; the device keeps its previous values.', true);
    } else if (reports.length) {
      status('Read back after the write and kept.');
    }
  };

  const containers = {};

  const reports = () => document.getElementById('device-report');

  const encodeSection = (encode) => {
    const holder = node('div', {});
    holder.append(node('h3', { class: 'set-h', text: 'Encoder' }));
    for (const stream of STREAMS) {
      const format = encode[stream] || {};
      const fps = number(format.fps);
      const bitrate = number(format.bitrate);
      const gop = number(format.gop);
      const resolution = text(format.resolution, '3072*2048');
      const save = node('button', { class: 'btn btn-fill', type: 'button', text: 'Save ' + stream });
      save.addEventListener('click', async () => {
        const patch = {};
        if (asNumber(fps) !== (format.fps === null || format.fps === undefined ? null : Number(format.fps))) {
          patch.fps = String(asNumber(fps));
        }
        if (asNumber(bitrate) !== (format.bitrate === null || format.bitrate === undefined ? null : Number(format.bitrate))) {
          patch.bitrate = String(asNumber(bitrate));
        }
        if (asNumber(gop) !== (format.gop === null || format.gop === undefined ? null : Number(format.gop))) {
          patch.gop = String(asNumber(gop));
        }
        if (resolution.value !== (format.resolution || '')) {
          patch.resolution = resolution.value;
        }
        if (!Object.keys(patch).length) {
          status('Nothing changed on the ' + stream + ' stream.');
          return;
        }
        status('Writing the ' + stream + ' encoder\u2026');
        const result = await api(BASE + '/encode?' + new URLSearchParams({ stream, ...patch }), { method: 'PUT' });
        if (!result.ok && !(result.body && result.body.reports)) {
          status('Could not write: ' + failureText(result), true);
          return;
        }
        renderReports(reports(), result.body);
        await load();
      });
      holder.append(
        node('div', { class: 'row' }, [
          node('div', { class: 'lab', text: stream + ' stream' }, [
            node('div', {
              class: 'hint',
              text:
                (format.compression || 'unknown codec') +
                ' \u00b7 quality ' + (format.quality === null || format.quality === undefined ? '?' : format.quality) +
                ' \u00b7 ' + (format.bitrate_control || '?'),
            }),
          ]),
          node('div', { class: 'ctl-col' }, [
            row('Frames per second', '', fps),
            row('Bitrate (kbps)', '', bitrate),
            row('Keyframe interval', '', gop),
            row('Resolution', 'The device token, for example 3072*2048.', resolution),
            save,
          ]),
        ]),
      );
    }
    return holder;
  };

  const networkSection = (network) => {
    const holder = node('div', {});
    const dhcp = node('input', { type: 'checkbox', role: 'switch' });
    dhcp.checked = Boolean(network.dhcp);
    const dns = text(network.dns, '1.1.1.1');
    const spare = text(network.spare_dns, '8.8.8.8');
    const gateway = text(network.gateway);
    const submask = text(network.submask);
    const save = node('button', { class: 'btn btn-fill', type: 'button', text: 'Save network' });
    save.addEventListener('click', async () => {
      const patch = {};
      if (dhcp.checked !== Boolean(network.dhcp)) {
        patch.dhcp = String(dhcp.checked);
      }
      if (dns.value !== (network.dns || '')) {
        patch.dns = dns.value;
      }
      if (spare.value !== (network.spare_dns || '')) {
        patch.spare_dns = spare.value;
      }
      if (gateway.value !== (network.gateway || '')) {
        patch.gateway = gateway.value;
      }
      if (submask.value !== (network.submask || '')) {
        patch.submask = submask.value;
      }
      if (!Object.keys(patch).length) {
        status('Nothing changed on the network.');
        return;
      }
      status('Writing the network settings\u2026');
      const result = await api(BASE + '/network?' + new URLSearchParams(patch), { method: 'PUT' });
      if (!result.ok && !(result.body && result.body.reports)) {
        status('Could not write: ' + failureText(result), true);
        return;
      }
      renderReports(reports(), result.body);
      await load();
    });
    holder.append(node('h3', { class: 'set-h', text: 'Network' }));
    holder.append(
      row('Address', 'Read only here: a new address moves the camera away from the one this app reaches, so the route refuses it.', node('p', { class: 'val' }, [node('span', { class: 'code', text: network.host_ip || 'unknown' })])),
      row('MAC', '', node('p', { class: 'val' }, [node('span', { class: 'code', text: network.mac || 'unknown' })])),
      row('DHCP', '', node('label', { class: 'switchrow' }, [dhcp, node('span', { text: 'Take the address from DHCP' })])),
      row('DNS', '', dns),
      row('Spare DNS', '', spare),
      row('Gateway', '', gateway),
      row('Subnet mask', '', submask),
      save,
    );
    return holder;
  };

  const storageSection = (storage) => {
    const holder = node('div', {});
    const partitions = storage.partitions.length
      ? node('table', { class: 'tbl' }, [
          node('thead', {}, [
            node('tr', {}, [
              node('th', { scope: 'col', text: 'Card' }),
              node('th', { scope: 'col', text: 'Status' }),
              node('th', { scope: 'col', text: 'Total' }),
              node('th', { scope: 'col', text: 'Free' }),
              node('th', { scope: 'col', text: 'Oldest recording' }),
            ]),
          ]),
          node(
            'tbody',
            {},
            storage.partitions.map((part) =>
              node('tr', {}, [
                node('th', { scope: 'row', text: String(part.index) }),
                node('td', { text: part.current === true ? 'active' : String(part.status === null || part.status === undefined ? '?' : part.status) }),
                node('td', { class: 'code', text: part.total_space || '?' }),
                node('td', { class: 'code', text: part.remain_space || '?' }),
                node('td', { class: 'code', text: part.start || 'no recording' }),
              ]),
            ),
          ),
        ])
      : node('p', { class: 'val', text: 'The device reports no card in this camera.' });
    const overwrite = node('select', {});
    overwrite.append(
      node('option', { value: 'overwrite', text: 'Overwrite the oldest recording' }),
      node('option', { value: 'stop', text: 'Stop recording when full' }),
    );
    const currentOverwrite = (storage.overwrite || 'OverWrite') === 'StopRecord' ? 'stop' : 'overwrite';
    overwrite.value = currentOverwrite;
    const record = node('select', {});
    record.append(
      node('option', { value: 'config', text: 'Follow the schedule' }),
      node('option', { value: 'manual', text: 'Always record' }),
      node('option', { value: 'off', text: 'Stop recording' }),
    );
    const wire = (storage.record_mode || 'ConfigRecord').toLowerCase();
    const currentRecord = wire.startsWith('manual') ? 'manual' : wire.startsWith('off') ? 'off' : 'config';
    record.value = currentRecord;
    const save = node('button', { class: 'btn btn-fill', type: 'button', text: 'Save storage' });
    save.addEventListener('click', async () => {
      const patch = {};
      if (overwrite.value !== currentOverwrite) {
        patch.overwrite = overwrite.value;
      }
      if (record.value !== currentRecord) {
        patch.record = record.value;
      }
      if (!Object.keys(patch).length) {
        status('Nothing changed on storage.');
        return;
      }
      status('Writing the storage settings\u2026');
      const result = await api(BASE + '/storage?' + new URLSearchParams(patch), { method: 'PUT' });
      if (!result.ok && !(result.body && result.body.reports)) {
        status('Could not write: ' + failureText(result), true);
        return;
      }
      renderReports(reports(), result.body);
      await load();
    });
    holder.append(node('h3', { class: 'set-h', text: 'Storage' }));
    holder.append(
      row('Memory card', '', partitions),
      row('When the card is full', '', overwrite),
      row('Record mode', '', record),
      save,
    );
    return holder;
  };

  const load = async () => {
    status('Reading the device settings over DVRIP\u2026');
    const [encode, network, storage] = await Promise.all([
      api(BASE + '/encode'),
      api(BASE + '/network'),
      api(BASE + '/storage'),
    ]);
    if (!encode.ok && !network.ok && !storage.ok) {
      status('The camera answered nothing: ' + failureText(encode), true);
      return;
    }
    const blocks = [
      ['encode', encode, encodeSection],
      ['network', network, networkSection],
      ['storage', storage, storageSection],
    ];
    for (const [name, result, render] of blocks) {
      const target = containers[name];
      if (!target) {
        continue;
      }
      if (result.ok && result.body) {
        target.replaceChildren(render(result.body[name]));
      } else if (result.body && result.body.message) {
        target.replaceChildren(node('p', { class: 'hint warn-text', text: name + ': ' + result.body.message }));
      } else {
        target.replaceChildren(node('p', { class: 'hint warn-text', text: name + ': ' + failureText(result) }));
      }
    }
    status('Read from the device just now.');
  };

  const open = () => {
    if (typeof window.openSettings === 'function') {
      window.openSettings('device');
      return;
    }
    if (typeof window.showSection === 'function') {
      window.showSection('device', true);
      return;
    }
    location.hash = '#settings/device';
  };

  const mount = () => {
    const nav = document.querySelector('.set-nav');
    const body = document.getElementById('set-body');
    if (!nav || !body || document.getElementById('device-encode')) {
      return false;
    }
    const link = node('a', { href: '#settings/device', text: 'Device settings', dataset: { section: 'device' } });
    link.addEventListener('click', (event) => {
      event.preventDefault();
      open();
    });
    nav.append(link);
    const section = node('section', { dataset: { section: 'device' }, 'aria-labelledby': 'h-device', hidden: '' }, [
      node('h2', { class: 'set-h', id: 'h-device', tabindex: '-1', text: 'Device settings' }),
      node('p', {
        class: 'set-sub',
        text: 'Encoder, network and storage as the camera holds them now, read over DVRIP. Every write saves a copy of the block first, reads it back, and puts the copy back when the device returns a different value.',
      }),
      node('p', { class: 'hint', id: 'device-status', role: 'status', text: 'Not loaded yet.' }),
      node('div', { id: 'device-report' }),
      node('div', {}, [
        node('div', { id: 'device-encode' }),
        node('div', { id: 'device-network' }),
        node('div', { id: 'device-storage' }),
      ]),
    ]);
    body.append(section);
    containers.encode = document.getElementById('device-encode');
    containers.network = document.getElementById('device-network');
    containers.storage = document.getElementById('device-storage');
    return true;
  };

  const start = () => {
    if (!mount()) {
      return;
    }
    const refresh = () => {
      if (location.hash === '#settings/device' || !document.querySelector('#set-body > section[data-section="device"]').hidden) {
        load();
      }
    };
    document.querySelectorAll('.set-nav a[data-section="device"]').forEach((link) => {
      link.addEventListener('click', () => load());
    });
    window.addEventListener('hashchange', refresh);
    window.addEventListener('load', refresh);
    refresh();
  };

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', start);
  } else {
    start();
  }

  return { load };
})();
