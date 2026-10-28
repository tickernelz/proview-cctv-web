(async () => {
  const until = async (check, what, ms = 20000) => {
    const end = Date.now() + ms;
    for (;;) {
      const value = check();
      if (value) return value;
      if (Date.now() > end) throw new Error('timed out waiting for ' + what);
      await new Promise((done) => setTimeout(done, 100));
    }
  };
  const shown = (node) => Boolean(node) && !node.hidden && node.getBoundingClientRect().height > 0;
  const opener = await until(
    () =>
      ['open-settings', 'empty-settings', 'login-settings']
        .map((id) => document.getElementById(id))
        .find(shown),
    'a Settings button',
  );
  opener.click();
  const configuration = await until(
    () => document.querySelector('.set-nav a[data-section="config"]'),
    'the Configuration link',
  );
  configuration.click();
  const input = await until(() => {
    for (const row of document.querySelectorAll('#settings-fields .row')) {
      const label = row.querySelector('.lab');
      if (label && label.textContent.includes('Needs a restart')) {
        return row.querySelector('input, select');
      }
    }
    return null;
  }, 'a field that needs a restart');
  input.value = input.value === '127.0.0.1:9797' ? '127.0.0.1:9798' : '127.0.0.1:9797';
  input.dispatchEvent(new Event('input', { bubbles: true }));
  const save = await until(() => {
    const button = document.getElementById('settings-save');
    return shown(button) ? button : null;
  }, 'the Save button');
  save.click();
  const restart = await until(() => {
    const button = document.getElementById('settings-restart');
    return shown(button) ? button : null;
  }, 'the Restart button');
  restart.click();
})().catch((error) => {
  const status = document.getElementById('settings-status');
  if (status) {
    status.textContent = 'Probe failed: ' + (error && error.message ? error.message : error);
  }
  console.error(error);
});
