
const CAMERAS = [
  { id: 'gate', name: 'Gate camera', ch: 1, lenses: 2, online: true, img: 'gate-close', lensLabel: 'dual lens' },
  { id: 'front', name: 'Front door', ch: 1, lenses: 1, online: true, img: 'front-door', lensLabel: 'fixed lens' },
  { id: 'garage', name: 'Garage', ch: 1, lenses: 1, online: true, img: 'garage-night', lensLabel: 'fixed lens' },
  { id: 'yard', name: 'Backyard', ch: 1, lenses: 1, online: true, img: 'yard', lensLabel: 'fixed lens' },
  { id: 'shop', name: 'Shop counter', ch: 1, lenses: 1, online: false, img: 'shop', lensLabel: 'fixed lens' }
];
let THEME = 'dark';
const byId = (id) => CAMERAS.find((c) => c.id === id);
const ic = (name, cls = '') => '<svg class="i ' + cls + '" aria-hidden="true"><use href="#i-' + name + '"></use></svg>';
const pic = (name) => '<img src="../img/' + name + '.jpg" alt="">';
const dot = (cam) => '<span class="dot ' + (cam.online ? '' : 'dot--off') + '"></span>';
const NOW = '21:40:12';

function switcher(cam, single) {
  return '<div class="switcher' + (single ? ' static' : '') + '">' + dot(cam) +
    '<span class="name">' + cam.name + '</span>' +
    '<span class="meta">channel ' + cam.ch + ' · ' + cam.lensLabel + '</span>' +
    (single ? '' : ic('chevron-down', 'i-16 chev')) + '</div>';
}

function switcherOpen(cam) {
  const rows = CAMERAS.map((c, n) =>
    '<button class="menu-row' + (c.id === cam.id ? ' sel' : '') + '">' + dot(c) +
      '<span class="col"><span class="name">' + c.name + '</span><span class="sub">' +
      (c.online ? 'channel ' + c.ch + ' · ' + c.lensLabel : 'offline since 21:12') + '</span></span>' +
      '<span class="meta">' + (n + 1) + '</span>' +
    '</button>').join('');
  return '<div class="switcher" aria-expanded="true">' + dot(cam) +
    '<span class="name">' + cam.name + '</span>' +
    '<span class="meta">channel ' + cam.ch + ' · ' + cam.lensLabel + '</span>' + ic('chevron-up', 'i-16 chev') +
    '<div class="menu" role="menu"><div class="menu-h">Cameras · 1-5 on the keyboard</div>' + rows +
      '<div class="menu-sep"></div>' +
      '<button class="menu-row">' + ic('refresh', 'i-16') + '<span class="name">Refresh the list</span></button>' +
      '<button class="menu-row">' + ic('sliders', 'i-16') + '<span class="name">Add a camera</span><span class="meta">Settings</span></button>' +
    '</div></div>';
}

function topbar(left, right) {
  return '<header class="topbar">' + left + '<div class="spacer"></div>' + right + '</header>';
}

function liveTopbar(cam, opts = {}) {
  const themeIcon = THEME === 'light' ? 'moon' : 'sun';
  const right = '<button class="iconbtn" aria-label="' + (opts.grid ? 'Back to the camera' : 'Show every camera') + '">' + ic(opts.grid ? 'frame' : 'grid') + '</button>' +
    '<button class="iconbtn" aria-label="Settings">' + ic('sliders') + '</button>' +
    '<button class="iconbtn" aria-label="Theme">' + ic(themeIcon) + '</button>';
  const left = opts.grid
    ? '<div class="switcher static"><span class="name">All cameras</span><span class="meta">4</span></div>'
    : (opts.menu ? switcherOpen(cam) : switcher(cam, opts.single));
  return topbar(left, right);
}

function caption(cam, state, right) {
  return '<div class="well-meta"><p><strong>' + cam.name + '</strong><span>' + state + '</span></p>' +
    '<div class="right">' + (right || '') + '<span class="mono">' + NOW + '</span></div></div>';
}

function controls(opts = {}) {
  const o = Object.assign({ talk: false, sound: false, record: false, snap: true, lens: 'Bottom lens', quality: 'HD', level: false }, opts);
  const lens = o.lens === false ? '' : '<button class="pick">' + o.lens + ic('chevron-down', 'i-16') + '</button><span class="sep"></span>';
  const snap = o.snap ? '<button class="ctl">' + ic('camera') + 'Snapshot</button>' : '';
  return '<div class="controls" role="toolbar" aria-label="Stream controls">' +
    '<button class="ctl' + (o.talk ? ' talk-on' : '') + '">' + ic('mic') + 'Talk' + (o.level ? '<span class="level"><i></i><i></i><i></i><i></i><i></i></span>' : '') + '</button>' +
    '<button class="ctl' + (o.sound ? ' is-on' : '') + '" aria-pressed="' + o.sound + '">' + ic(o.sound ? 'speaker' : 'speaker-off') + 'Sound</button>' +
    '<span class="sep"></span>' +
    '<button class="ctl' + (o.record ? ' rec-on' : '') + '">' + ic('record') + (o.record ? '<span class="mono">00:42</span>' : 'Record') + '</button>' + snap +
    '<span class="spacer"></span>' + (o.ptz ? '<button class="ctl" aria-pressed="' + (o.ptz === 'open') + '">' + ic('move') + 'Move</button>' : '') + lens +
    '<button class="pick">Quality ' + o.quality + ic('chevron-down', 'i-16') + '</button>' +
    '<span class="sep"></span>' +
    '<button class="iconbtn" aria-label="Full screen">' + ic('expand') + '</button>' +
  '</div>';
}

function banner(text, sub) {
  return '<div class="banner">' + ic('disconnect', 'i-16') + '<span><strong>' + text + '</strong> ' + sub + '</span><div class="spacer"></div><button>Retry now</button></div>';
}

function shell(o) {
  return '<div class="app' + (o.cls ? ' ' + o.cls : '') + '">' + o.topbar + (o.banner || '') + o.body + '</div>';
}

function live(cam, o = {}) {
  return '<div class="viewer"><div class="well">' + (o.well || pic(cam.img)) + '</div>' +
    caption(cam, o.state || (cam.lenses > 1 ? 'live, bottom lens, 0.4 s behind' : 'live, 0.4 s behind'), o.right) +
    controls(o.controls || {}) + '</div>';
}

function tile(cam, o = {}) {
  const meta = (state, time, cls) => '<div class="tile-meta"><p><strong>' + cam.name + '</strong><span>' + state + '</span></p>' +
    '<span class="right ' + (cls || '') + '">' + time + '</span></div>';
  if (!cam.online) {
    return '<button class="tile' + (o.sel ? ' sel' : '') + '"><div class="well"><div class="tile-fail">' +
      '<strong>Offline</strong><span>The server tries again every 30 seconds.</span></div></div>' +
      meta('offline since 21:12', 'last frame 21:11:58', 'warn') + '</button>';
  }
  if (o.fail) {
    return '<button class="tile"><div class="well"><div class="tile-fail">' +
      '<strong>No picture</strong><span>The camera refused the stream: RTSP 401.</span>' +
      '<span class="link">Try again</span></div></div>' + meta('failed', '21:40:08', 'warn') + '</button>';
  }
  return '<button class="tile' + (o.sel ? ' sel' : '') + '"><div class="well">' + pic(cam.img) + '</div>' +
    meta('live', NOW) + '</button>';
}

const SCREENS = {
  signin: () => auth({
    lede: 'The same PROVIEW account your phone uses. The camera password stays on this machine.',
    stage: 'The picture appears here as soon as a camera answers.'
  }),
  signin_error: () => auth({
    lede: 'The same PROVIEW account your phone uses. The camera password stays on this machine.',
    stage: 'The picture appears here as soon as a camera answers.',
    error: 'The account or password was refused by api.jftechws.com.',
    email: 'zhafron@example.com', pw: 'wrong-password'
  }),
  signin_expired: () => auth({
    lede: 'The server ended this session at 21:12, while the window was in the background.',
    stage: 'Last frame 21:12:04. Signing in picks the stream up again.',
    expired: true
  }),
  devices_empty: () => shell({
    topbar: liveTopbar({ name: 'No camera', ch: 0, lensLabel: 'nothing selected', online: false }),
    body: '<div class="emptywrap"><div class="empty">' +
      '<h2>No cameras on this account yet</h2>' +
      '<p>Add the camera in the phone app with the same account, then refresh. If the camera is on your own network, set its address under Settings, Configuration instead.</p>' +
      '<div class="actions"><button class="btn btn-line">' + ic('refresh', 'i-16') + 'Refresh</button><button class="btn btn-text">Open Settings</button></div>' +
      '</div></div>'
  }),
  live: () => shell({ topbar: liveTopbar(byId('gate')), body: live(byId('gate'), { controls: { ptz: true } }) }),
  live_switcher: () => shell({ topbar: liveTopbar(byId('gate'), { menu: true }), body: live(byId('gate')) }),
  live_loading: () => shell({
    topbar: liveTopbar(byId('front')),
    body: '<div class="viewer"><div class="well"><div class="prog"></div><div class="well-empty">' +
      '<h2>Requesting the stream</h2><p>Front door, channel 1, HD. The server is asking the camera for a ticket.</p></div></div>' +
      caption(byId('front'), 'connecting — nothing has been received yet', '<span>not live</span>') +
      controls({ snap: false }) + '</div>'
  }),
  live_fail: () => shell({
    topbar: liveTopbar(byId('garage')),
    body: '<div class="viewer"><div class="well"><div class="well-fail">' +
      '<h2>No picture from Garage</h2>' +
      '<p>The camera refused the stream. This is the server\u2019s own answer:</p>' +
      '<p class="mono">RTSP 401 Unauthorized · 192.168.1.71:554</p>' +
      '<div class="actions"><button class="btn btn-line inv">' + ic('refresh', 'i-16') + 'Try again</button>' +
      '<button class="btn btn-text inv">Check the camera password</button></div></div></div>' +
      caption(byId('garage'), 'failed at 21:40:08 — 4 seconds after the request', '') +
      controls({ snap: false }) + '</div>'
  }),
  live_stalled: () => shell({
    topbar: liveTopbar(byId('gate')),
    body: live(byId('gate'), { state: 'frozen 12 s — the frame below is from 21:39:58', right: '<span class="warn">not live</span>' })
  }),
  live_offline: () => shell({
    topbar: liveTopbar(byId('shop')),
    banner: banner('Shop counter has not answered since 21:12.', 'The server keeps trying every 30 seconds.'),
    body: '<div class="viewer"><div class="well"><div class="well-empty"><h2>Camera offline</h2>' +
      '<p>Last frame 21:11:58. The picture comes back on its own when the camera answers.</p></div></div>' +
      caption(byId('shop'), 'offline for 28 minutes', '<span class="warn">offline</span>') +
      controls({ snap: false }) + '</div>'
  }),
  nonetwork: () => shell({
    topbar: liveTopbar(byId('gate')),
    banner: banner('This computer lost its network at 21:38:55.', 'What you see below is the last frame the app received.'),
    body: live(byId('gate'), { state: 'not updating — last frame 21:38:55', right: '<span class="warn">stale</span>' })
  }),
  live_single: () => shell({
    topbar: liveTopbar(byId('front'), { single: true, grid: false }).replace('aria-label="Show every camera"', 'aria-label="Show every camera" disabled'),
    body: live(byId('front'), { controls: { lens: false } })
  }),
  lens_menu: () => shell({
    topbar: liveTopbar(byId('gate')),
    body: live(byId('gate'), { well: pic('gate-stacked'), state: 'live, both lenses as the camera stacks them', controls: { lens: 'Both lenses', ptz: true } }).replace('<span class="spacer"></span>',
      '<span class="spacer"></span><div class="popover lens-pop" role="menu">' +
      '<div class="menu-h">Lens · L cycles</div>' +
      '<button class="menu-row sel">' + ic('check', 'i-16') + '<span class="col"><span class="name">Both lenses</span><span class="sub">the frame as the camera sends it</span></span><span class="meta">1:1</span></button>' +
      '<button class="menu-row"><span class="i-16"></span><span class="col"><span class="name">Top lens</span><span class="sub">wide · fixed, aimed by hand</span></span></button>' +
      '<button class="menu-row"><span class="i-16"></span><span class="col"><span class="name">Bottom lens</span><span class="sub">pan and tilt from here</span></span></button>' +
      '<div class="menu-sep"></div><p class="menu-note">Split inferred from the frame shape. The server has not reported the lens layout yet.</p>' +
      '</div>')
  }),
  ptz: () => shell({
    topbar: liveTopbar(byId('gate')),
    body: live(byId('gate'), {
      state: 'live, bottom lens — arrow keys move it',
      well: pic('gate-close') + '<div class="pad" role="group" aria-label="Pan and tilt">' +
        '<button class="pk up" aria-label="Tilt up">' + ic('chevron-up') + '</button>' +
        '<button class="pk left" aria-label="Pan left">' + ic('chevron-left') + '</button>' +
        '<button class="pk home" aria-label="Stop">' + ic('stop', 'i-16') + '</button>' +
        '<button class="pk right" aria-label="Pan right">' + ic('chevron-right') + '</button>' +
        '<button class="pk down" aria-label="Tilt down">' + ic('chevron-down') + '</button>' +
        '<p class="pad-note">Preset<span>Gate · Driveway · Porch · Street</span></p></div>',
      controls: { lens: 'Bottom lens', ptz: 'open' }
    })
  }),
  boot: () => shell({
    topbar: topbar('<div class="sk" style="width:220px;height:20px"></div>', '<div class="sk" style="width:116px;height:28px"></div>'),
    body: '<div class="viewer"><div class="well"></div>' +
      '<div class="well-meta"><div class="sk" style="width:260px;height:14px"></div></div>' +
      '<div class="controls"><div class="sk" style="width:300px;height:28px"></div><span class="spacer"></span><div class="sk" style="width:200px;height:28px"></div></div></div>'
  }),
  lenses: () => shell({
    topbar: liveTopbar(byId('gate')),
    body: live(byId('gate'), { well: pic('street-wide'), state: 'live, top lens (fixed), crop inferred from the frame', controls: { lens: 'Top lens', ptz: false } })
  }),
  fullscreen: () => shell({
    topbar: liveTopbar(byId('gate')),
    cls: 'is-full',
    body: '<div class="viewer fs"><div class="well">' + pic('gate-close') +
      '<div class="fs-line"><span>Gate camera · bottom lens</span><span>live · ' + NOW + '</span></div></div></div>'
  }),
  talk: () => shell({
    topbar: liveTopbar(byId('gate')),
    body: live(byId('gate'), { state: 'live — the microphone is open', right: '<span>release to stop</span>', controls: { talk: true, level: true } })
  }),
  record: () => shell({
    topbar: liveTopbar(byId('gate')),
    body: live(byId('gate'), { right: '<span class="rec"><span class="dot dot--rec"></span>Recording 00:42</span>', controls: { record: true } })
  }),
  grid: () => shell({
    topbar: liveTopbar(byId('gate'), { grid: true }),
    body: '<div class="viewer"><div class="tiles">' +
      tile(byId('front')) + tile(byId('garage')) + tile(byId('yard'), { sel: true }) + tile(byId('shop')) +
      '</div><div class="controls"><p class="count">4 cameras · 3 live · 1 offline</p><span class="spacer"></span>' +
      '<button class="pick">Layout Auto' + ic('chevron-down', 'i-16') + '</button><span class="sep"></span>' +
      '<button class="iconbtn" aria-label="Full screen">' + ic('expand') + '</button></div></div>'
  }),
  grid2: () => shell({
    topbar: liveTopbar(byId('gate'), { grid: true }),
    body: '<div class="viewer"><div class="tiles two">' + tile(byId('front'), { sel: true }) + tile(byId('garage')) +
      '</div><div class="controls"><p class="count">2 cameras · 2 live</p><span class="spacer"></span>' +
      '<button class="pick">Layout Auto' + ic('chevron-down', 'i-16') + '</button><span class="sep"></span>' +
      '<button class="iconbtn" aria-label="Full screen">' + ic('expand') + '</button></div></div>'
  }),
  grid_fail: () => shell({
    topbar: liveTopbar(byId('gate'), { grid: true }),
    body: '<div class="viewer"><div class="tiles">' +
      tile(byId('front'), { sel: true }) + tile(byId('garage'), { fail: true }) + tile(byId('yard')) + tile(byId('shop')) +
      '</div><div class="controls"><p class="count">4 cameras · 2 live · 1 failed · 1 offline</p><span class="spacer"></span>' +
      '<button class="pick">Layout Auto' + ic('chevron-down', 'i-16') + '</button><span class="sep"></span>' +
      '<button class="iconbtn" aria-label="Full screen">' + ic('expand') + '</button></div></div>'
  }),
  settings_playback: () => settings('Playback', settingsPlayback(), false),
  settings_device: () => settings('This camera', settingsDevice(), false),
  settings_config: () => settings('Configuration', settingsConfig(), true)
};

function auth(o) {
  const top = topbar('<span class="brand"><span class="mark"></span>PROVIEW</span>',
    '<button class="iconbtn" aria-label="Theme">' + ic(THEME === 'light' ? 'moon' : 'sun') + '</button>');
  const notice = o.expired
    ? '<div class="notice">' + ic('clock', 'i-16') + '<span>Nothing is being recorded while you are signed out.</span></div>'
    : '';
  const err = o.error
    ? '<div class="err">' + ic('warning', 'i-16') + '<span>' + o.error + '</span></div>'
    : '';
  return '<div class="app">' + top +
    '<div class="auth"><div class="auth-form">' +
      '<h1>Watch the camera from this desk</h1>' +
      '<p class="lede">' + o.lede + '</p>' + notice +
      '<div class="tabs"><button aria-selected="true">Account</button><button aria-selected="false">This network</button></div>' +
      '<form><div class="field"><label for="e">Email</label><input id="e" value="' + (o.email || '') + '" placeholder="name@example.com"></div>' +
      '<div class="field' + (o.error ? ' bad' : '') + '"><label for="p">Password</label><input id="p" type="password" value="' + (o.pw || '') + '">' + err + '</div>' +
      '<button class="btn btn-fill">' + (o.expired ? 'Sign in to resume' : 'Sign in') + '</button></form>' +
      '<p class="auth-note">Settings opens before you sign in: cloud address, camera address, camera password.</p>' +
    '</div>' +
    '<div class="auth-stage"><div class="stage"></div><p class="stage-cap">' + o.stage + '</p></div>' +
    '</div></div>';
}

function settings(title, body, dirty) {
  const top = topbar('<button class="ctl back">' + ic('arrow-left', 'i-16') + 'Live</button><span class="screen-title">Settings</span>',
    '<button class="iconbtn" aria-label="Theme">' + ic(THEME === 'light' ? 'moon' : 'sun') + '</button>');
  const nav = ['Playback', 'This camera', 'Connection', 'Configuration', 'Server'].map((n) =>
    '<a href="#"' + (n === title ? ' aria-current="page"' : '') + '>' + n + '</a>').join('');
  const bar = '<div class="savebar"><span class="state">' + (dirty ? 'Unsaved changes · 2 fields' : 'Everything saved') + '</span>' +
    '<div class="spacer"></div>' +
    (dirty ? '<button class="btn btn-text">Discard</button><button class="btn btn-fill" style="width:auto;padding:0 16px">Save changes</button>' : '') +
    '</div>';
  return '<div class="app">' + top + '<div class="set"><nav class="set-nav">' + nav + '</nav><div class="set-body">' + body + '</div></div>' + bar + '</div>';
}

function settingsPlayback() {
  const opt = (label, on) => '<div class="opt" aria-checked="' + on + '" role="radio"><span class="box"></span>' + label + '</div>';
  const sw = (label, on) => '<div class="switchrow"><span class="sw" aria-checked="' + on + '" role="switch"></span>' + label + '</div>';
  return '<h1 class="set-h">Playback</h1>' +
    '<p class="set-sub">What happens when a camera opens. These live in this browser, not on the server.</p>' +
    '<div class="row"><div class="lab">Stream quality</div><div class="ctl-col">' + opt('HD — the main stream', true) + opt('SD — the sub stream, lighter on the network', false) + '</div></div>' +
    '<div class="row"><div class="lab">Lens on open<div class="hint">Only the Gate camera has two.</div></div><div class="ctl-col">' + opt('Both lenses', true) + opt('Top — wide', false) + opt('Bottom — pan and tilt', false) + '</div></div>' +
    '<div class="row"><div class="lab">Sound</div><div class="ctl-col">' + sw('Start muted', true) + sw('Keep the audio open in the background', false) + '</div></div>' +
    '<div class="row"><div class="lab">Give up after<div class="hint">A stream that sends nothing for this long is reported as failed instead of waiting.</div></div><div class="ctl-col"><div class="selbox">6 seconds' + ic('chevron-down', 'i-16 chev') + '</div></div></div>' +
    '<div class="row"><div class="lab">Time on the picture<div class="hint">Burns the clock into snapshots and recordings. It stays out of the live picture either way.</div></div><div class="ctl-col">' + sw('Burn the clock into saved files', true) + '</div></div>';
}

function settingsDevice() {
  const rows = [
    ['Pan and tilt', 'Bottom lens', 'supported', 'DVRIP OP/PTZ · 8 directions, 300 ms per step'],
    ['Presets', 'Bottom lens', 'supported', '4 presets the camera lists by name'],
    ['Optical zoom', 'Bottom lens', 'unavailable', 'The report has no zoom row for this lens'],
    ['Auto-tracking', '—', 'unavailable', 'Not in the report'],
    ['White light', '—', 'supported', 'Two modes: on, off'],
    ['Speaker', '—', 'unavailable', 'No audio output row'],
    ['Two-way audio', '—', 'unavailable', 'Needs the speaker row above'],
    ['SD card', '—', 'unavailable', 'No storage row in the report']
  ].map((r) => '<tr><td>' + r[0] + '</td><td>' + r[1] + '</td><td>' +
    (r[2] === 'supported' ? '<span class="pill ok">supported</span>' : '<span class="pill no">unavailable</span>') +
    '</td><td>' + r[3] + '</td></tr>').join('');
  return '<h1 class="set-h">This camera</h1>' +
    '<p class="set-sub">What the device says about itself, as the server last read it. An unavailable row means the camera returned no evidence, not that the app forgot to look.</p>' +
    '<div class="row"><div class="lab">Name</div><div class="ctl-col"><input class="text-input" value="Gate camera"></div></div>' +
    '<div class="row"><div class="lab">Device id<div class="hint">The same id the phone app shows.</div></div><div class="ctl-col"><span class="code">XM-IPG-5C4E9A02</span><span class="code">channel 1</span></div></div>' +
    '<div class="row"><div class="lab">Connection</div><div class="ctl-col"><p class="val">Local network · 192.168.1.64</p><p class="hint">DVRIP 34567 · RTSP 554 · last answered ' + NOW + '</p></div></div>' +
    '<div class="row"><div class="lab">What it can do</div><div class="ctl-col"><table class="tbl"><thead><tr><th>Feature</th><th>Lens</th><th>Reported</th><th>Evidence</th></tr></thead><tbody>' + rows + '</tbody></table></div></div>' +
    '<div class="row"><div class="lab">Remove</div><div class="ctl-col"><p class="hint">Hiding a camera here leaves it on the account and in the phone app.</p><button class="btn btn-text danger">Remove from this app</button></div></div>';
}

function settingsConfig() {
  return '<h1 class="set-h">Configuration</h1>' +
    '<p class="set-sub">Server settings, stored on this machine. Saving sends only the fields you changed. A key that needs a restart says so in its own label.</p>' +
    '<div class="row"><div class="lab">Provider</div><div class="ctl-col"><div class="selbox">Local network' + ic('chevron-down', 'i-16 chev') + '</div><p class="hint">Cloud account, this network, or the built-in fixture stream.</p></div></div>' +
    '<div class="row"><div class="lab">Cloud address</div><div class="ctl-col"><input class="text-input mono-input" value="https://api.jftechws.com"></div></div>' +
    '<div class="row"><div class="lab">Camera address</div><div class="ctl-col"><input class="text-input mono-input" value="192.168.1.64:34567"></div></div>' +
    '<div class="row"><div class="lab">Camera password<div class="hint">Stored on the server. Nothing reads it back into this page.</div></div><div class="ctl-col"><input class="text-input" placeholder="stored, type to replace" value=""></div></div>' +
    '<div class="row"><div class="lab">Listen address<div class="hint">Needs a restart.</div></div><div class="ctl-col"><input class="text-input mono-input" value="127.0.0.1:8787"></div></div>' +
    '<div class="row"><div class="lab">Data folder</div><div class="ctl-col"><input class="text-input mono-input" value="~/.local/share/proview"></div></div>';
}

function render() {
  const p = new URLSearchParams(location.search);
  const screen = (p.get('screen') || 'live').replace(/-/g, '_');
  const theme = p.get('theme') || 'dark';
  THEME = theme;
  document.documentElement.dataset.theme = theme;
  const fn = SCREENS[screen] || SCREENS.live;
  document.getElementById('root').innerHTML = fn();
  document.title = screen + ' · ' + theme;
}
render();
