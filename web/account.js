(function () {
  'use strict';

  const el = (id) => document.getElementById(id);

  async function call(path, options) {
    const response = await fetch(path, Object.assign({ headers: { 'content-type': 'application/json' } }, options || {}));
    let body = null;
    try {
      body = await response.json();
    } catch (error) {
      body = null;
    }
    if (!response.ok) {
      const failure = new Error((body && body.message) || response.statusText);
      failure.status = response.status;
      failure.code = body && body.code;
      throw failure;
    }
    return body;
  }

  function rememberBox() {
    const form = el('login-form');
    if (!form || el('remember-account')) {
      return el('remember-account');
    }
    const field = document.createElement('div');
    field.className = 'field';
    field.innerHTML =
      '<label style="display:flex;gap:8px;align-items:center;font-size:var(--t-14)"><input id="remember-account" type="checkbox" style="height:16px;width:16px"> <span>Remember this account on this machine</span></label>' +
      '<p class="hint" id="remember-note" hidden></p>';
    const submit = el('login-submit');
    form.insertBefore(field, submit || null);
    el('remember-account').addEventListener('change', () => {
      el('remember-note').hidden = true;
    });
    return el('remember-account');
  }

  function setNote(text) {
    const note = el('remember-note');
    if (!note) {
      return;
    }
    note.textContent = text;
    note.hidden = !text;
  }

  async function rememberNow() {
    const box = el('remember-account');
    if (!box || !box.checked) {
      return;
    }
    try {
      await call('/api/account/remember', { method: 'POST' });
      setNote('This account will come back signed in next time.');
    } catch (error) {
      setNote('Could not remember the account: ' + error.message);
    }
  }

  async function restore() {
    try {
      const state = await call('/api/account/remembered');
      const email = el('email');
      if (email && state.email && !email.value) {
        email.value = state.email;
      }
      if (state.email && !state.restored) {
        setNote('The remembered session for ' + state.email + ' expired. Sign in again to continue.');
      }
    } catch (error) {
      /* the sign-in screen still works without the remembered account */
    }
  }

  function watchSignIn() {
    const screen = el('signin');
    if (!screen) {
      return;
    }
    let wasVisible = !screen.hidden;
    const observer = new MutationObserver(() => {
      const visible = !screen.hidden;
      if (wasVisible && !visible) {
        rememberNow();
      }
      wasVisible = visible;
    });
    observer.observe(screen, { attributes: true, attributeFilter: ['hidden'] });
  }

  function panel() {
    if (el('add-device')) {
      return el('add-device');
    }
    const empty = document.querySelector('#empty .actions');
    if (!empty) {
      return null;
    }
    const section = document.createElement('section');
    section.id = 'add-device';
    section.className = 'add-device';
    section.innerHTML =
      '<details><summary>Add a camera by serial or QR code</summary>' +
      '<form id="add-device-form" novalidate>' +
      '<div class="field"><label for="add-sn">Serial number</label><input id="add-sn" type="text" spellcheck="false" autocomplete="off" placeholder="7f92970ddf5282743hrc"></div>' +
      '<div class="field"><label for="add-user">Camera username</label><input id="add-user" type="text" spellcheck="false" autocomplete="off" value="admin"></div>' +
      '<div class="field"><label for="add-pass">Camera password</label><input id="add-pass" type="password" autocomplete="off"></div>' +
      '<button class="btn btn-fill" id="add-device-submit" type="submit">Add this camera</button>' +
      '</form>' +
      '<form id="add-qr-form" novalidate>' +
      '<div class="field"><label for="add-qr">QR code payload or share link</label><input id="add-qr" type="text" spellcheck="false" autocomplete="off" placeholder="https://d.xmeye.net/CSee?shareInfo=..."></div>' +
      '<button class="btn btn-line" id="add-qr-submit" type="submit">Add the camera in that code</button>' +
      '</form>' +
      '<p class="hint" id="add-device-status" role="status" hidden></p>' +
      '</details>';
    empty.insertAdjacentElement('afterend', section);
    wireForms();
    return section;
  }

  function status(text) {
    const line = el('add-device-status');
    if (!line) {
      return;
    }
    line.textContent = text || '';
    line.hidden = !text;
  }

  function report(reply) {
    const sentence =
      reply.status === 'already_bound'
        ? reply.serial + ' was already on this account (' + reply.vendor_code + ')'
        : reply.serial + ' is now on this account';
    status(sentence);
    const refresh = el('refresh');
    if (refresh) {
      refresh.click();
    }
  }

  function wireForms() {
    const serial = el('add-device-form');
    if (serial) {
      serial.addEventListener('submit', async (event) => {
        event.preventDefault();
        const button = el('add-device-submit');
        button.disabled = true;
        status('Asking the cloud to bind the camera…');
        try {
          const reply = await call('/api/devices/bind', {
            method: 'POST',
            body: JSON.stringify({
              sn: el('add-sn').value.trim(),
              username: el('add-user').value.trim() || 'admin',
              password: el('add-pass').value,
            }),
          });
          el('add-pass').value = '';
          report(reply);
        } catch (error) {
          status(
            error.code === 'device_credentials'
              ? 'Not added: the camera refused that username and password. Check the label on the camera.'
              : 'Not added: ' + error.message,
          );
        } finally {
          button.disabled = false;
        }
      });
    }
    const qr = el('add-qr-form');
    if (qr) {
      qr.addEventListener('submit', async (event) => {
        event.preventDefault();
        const button = el('add-qr-submit');
        button.disabled = true;
        status('Reading the code…');
        try {
          const reply = await call('/api/devices/bind/qr', {
            method: 'POST',
            body: JSON.stringify({ payload: el('add-qr').value.trim() }),
          });
          el('add-qr').value = '';
          report(reply);
        } catch (error) {
          status(
            error.code === 'bad_qr'
              ? 'Not added: that code is not a PROVIEW device code. ' + error.message
              : 'Not added: ' + error.message,
          );
        } finally {
          button.disabled = false;
        }
      });
    }
  }

  function watchEmpty() {
    const empty = el('empty');
    if (!empty) {
      return;
    }
    const observer = new MutationObserver(() => {
      if (!empty.hidden) {
        panel();
      }
    });
    observer.observe(empty, { attributes: true, attributeFilter: ['hidden'] });
    if (!empty.hidden) {
      panel();
    }
  }

  function watchSignOut() {
    const button = el('signout');
    if (!button) {
      return;
    }
    button.addEventListener('click', () => {
      call('/api/account/signout', { method: 'POST' }).catch(() => {});
    });
  }

  function start() {
    rememberBox();
    restore();
    watchSignIn();
    watchEmpty();
    watchSignOut();
  }

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', start);
  } else {
    start();
  }
})();
