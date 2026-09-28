'use strict';

/* WebDock launcher.
 * Talks to Rust through `__TAURI_INTERNALS__.invoke` (no Tauri globals are
 * exposed to hosted apps); Rust pushes events via `window.webdock.onEvent`. */

const $ = (sel, root = document) => root.querySelector(sel);

// ------------------------------------------------------------------ backend

const tauri = window.__TAURI_INTERNALS__;
const invoke = tauri
  ? (cmd, args = {}) => tauri.invoke(cmd, args)
  : (cmd, args) => mockInvoke(cmd, args); // for UI work in a plain browser

// ------------------------------------------------------------------ state

const prefs = loadPrefs();
let overview = null;
let running = new Set();
let category = null;
let refreshTimer = null;

function loadPrefs() {
  const defaults = { view: 'grid', sort: 'smart', theme: 'system' };
  try {
    return Object.assign(defaults, JSON.parse(localStorage.getItem('webdock.prefs') || '{}'));
  } catch {
    return defaults;
  }
}
function savePrefs() {
  try { localStorage.setItem('webdock.prefs', JSON.stringify(prefs)); } catch { /* storage unavailable */ }
}

// ------------------------------------------------------------------ events from Rust

window.webdock = {
  onEvent(name, payload) {
    switch (name) {
      case 'apps-changed':
        clearTimeout(refreshTimer);
        refreshTimer = setTimeout(load, 80);
        break;
      case 'running':
        running = new Set(payload || []);
        render();
        break;
      case 'drag':
        $('#drop-overlay').hidden = !payload;
        break;
      case 'imported': {
        const ok = payload.filter((r) => r.ok).length;
        if (ok) toast(t('imported', ok));
        payload.filter((r) => !r.ok).forEach((r) => toast(`${t('importFailed')}: ${r.error}`, true));
        break;
      }
    }
  },
};

// ------------------------------------------------------------------ loading & rendering

async function load() {
  try {
    overview = await invoke('get_overview');
    running = new Set(overview.running);
    if (overview.notice) toast(overview.notice, true);
    render();
  } catch (e) {
    toast(String(e), true);
  }
}

function sortApps(list) {
  const byName = (a, b) => a.name.localeCompare(b.name, undefined, { numeric: true, sensitivity: 'base' });
  const sorted = [...list];
  switch (prefs.sort) {
    case 'name': return sorted.sort(byName);
    case 'recent': return sorted.sort((a, b) => b.last_opened - a.last_opened || byName(a, b));
    case 'usage': return sorted.sort((a, b) => b.launches - a.launches || byName(a, b));
    default: {
      // Running first, then a blend of frequency and recency, then name.
      const now = Date.now();
      const score = (a) => {
        if (!a.last_opened) return 0;
        const days = (now - a.last_opened) / 864e5;
        return Math.log2(1 + a.launches) / (1 + days / 7);
      };
      return sorted.sort((a, b) =>
        (running.has(b.id) - running.has(a.id)) || (score(b) - score(a)) || byName(a, b));
    }
  }
}

function matches(app, q) {
  if (!q) return true;
  const hay = [app.name, app.description, app.id, app.category, app.folder, app.author].join(' ').toLowerCase();
  // Every term must appear; also allow subsequence matching on the name ("vsc" → "VS Code").
  return q.split(/\s+/).every((term) => hay.includes(term) || subsequence(term, app.name.toLowerCase()));
}
function subsequence(needle, hay) {
  let i = 0;
  for (const c of hay) if (c === needle[i]) i++;
  return i === needle.length && needle.length > 1;
}

function render() {
  if (!overview) return;
  const q = $('#search').value.trim().toLowerCase();
  const apps = overview.apps;
  document.body.classList.toggle('list', prefs.view === 'list');
  $('#view-grid').setAttribute('aria-pressed', prefs.view === 'grid');
  $('#view-list').setAttribute('aria-pressed', prefs.view === 'list');
  $('#sort').value = prefs.sort;

  renderCategories(apps);
  const visible = sortApps(apps.filter((a) => matches(a, q) && (!category || (a.category || '') === category)));
  const pinned = q ? [] : visible.filter((a) => a.pinned);
  const rest = q ? visible : visible.filter((a) => !a.pinned);

  fillGrid($('#apps-pinned'), pinned);
  fillGrid($('#apps-all'), rest);
  $('#section-pinned').hidden = pinned.length === 0;
  $('#section-all').hidden = rest.length === 0;
  $('#all-title').hidden = pinned.length === 0 && !category;
  $('#all-title').textContent = category || t('allApps');
  $('#empty').hidden = apps.length !== 0;
  $('#no-results').hidden = apps.length === 0 || visible.length !== 0;
  $('#empty-path').textContent = overview.info.apps_dir;

  $('#brand-name').textContent = overview.info.title || 'WebDock';
  document.title = overview.info.title || 'WebDock';
  $('#status-count').textContent = t('countApps', apps.length, running.size);
  $('#status-path').textContent = overview.info.apps_dir;
}

function renderCategories(apps) {
  const cats = [...new Set(apps.map((a) => a.category || ''))].sort();
  const nav = $('#categories');
  if (cats.length < 2) {
    nav.hidden = true;
    category = null;
    return;
  }
  if (category !== null && !cats.includes(category)) category = null;
  nav.hidden = false;
  nav.replaceChildren(
    chip(t('all'), category === null, () => { category = null; render(); }),
    ...cats.map((c) => chip(c || t('uncategorized'), category === c, () => { category = c; render(); })),
  );
}
function chip(label, active, onClick) {
  const b = el('button', { className: 'chip', textContent: label });
  b.setAttribute('aria-pressed', active);
  b.addEventListener('click', onClick);
  return b;
}

function fillGrid(container, list) {
  const frag = document.createDocumentFragment();
  for (const app of list) frag.appendChild(card(app));
  container.replaceChildren(frag);
}

function card(app) {
  const c = el('div', { className: 'card', tabIndex: 0 });
  c.dataset.id = app.id;
  c.setAttribute('role', 'button');
  c.setAttribute('aria-label', app.name);
  if (running.has(app.id)) c.appendChild(el('span', { className: 'running-dot', title: t('running') }));
  c.appendChild(iconFor(app));

  const text = el('div', { className: 'text' });
  text.appendChild(el('div', { className: 'name', textContent: app.name, title: app.name }));
  if (app.description) text.appendChild(el('div', { className: 'desc', textContent: app.description }));
  c.appendChild(text);

  const meta = el('div', { className: 'meta' });
  if (app.mode === 'remote') meta.appendChild(el('span', { className: 'badge remote', textContent: t('remote') }));
  if (app.mode === 'localhost') meta.appendChild(el('span', { className: 'badge http', textContent: t('localhost') }));
  if (app.version) meta.appendChild(el('span', { className: 'badge', textContent: 'v' + app.version.replace(/^v/i, '') }));
  if (meta.childElementCount) c.appendChild(meta);

  const actions = el('div', { className: 'card-actions' });
  const pin = iconButton(app.pinned ? t('unpin') : t('pin'), ICONS.pin, (e) => { e.stopPropagation(); togglePin(app); });
  if (app.pinned) pin.classList.add('pinned');
  actions.append(pin, iconButton('…', ICONS.more, (e) => {
    e.stopPropagation();
    const r = e.currentTarget.getBoundingClientRect();
    showMenu(app, r.left, r.bottom + 4);
  }));
  c.appendChild(actions);

  c.addEventListener('click', (e) => openApp(app, e.ctrlKey || e.metaKey || e.shiftKey, c));
  c.addEventListener('auxclick', (e) => { if (e.button === 1) openApp(app, true, c); });
  c.addEventListener('contextmenu', (e) => { e.preventDefault(); showMenu(app, e.clientX, e.clientY); });
  c.addEventListener('keydown', (e) => {
    if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); openApp(app, e.ctrlKey || e.metaKey, c); }
    if (e.key === 'ContextMenu' || (e.shiftKey && e.key === 'F10')) {
      e.preventDefault();
      const r = c.getBoundingClientRect();
      showMenu(app, r.left + 20, r.top + 40);
    }
  });
  return c;
}

function iconFor(app) {
  const box = el('div', { className: 'icon' });
  const mono = () => {
    box.classList.add('mono');
    box.replaceChildren(document.createTextNode(initial(app.name)));
    const h = hash(app.id) % 360;
    box.style.background = app.theme_color && /^#[0-9a-f]{3,8}$/i.test(app.theme_color)
      ? `linear-gradient(135deg, ${app.theme_color}, hsl(${h} 70% 45%))`
      : `linear-gradient(135deg, hsl(${h} 75% 60%), hsl(${(h + 40) % 360} 70% 48%))`;
  };
  if (app.icon_url) {
    const img = el('img', { alt: '', loading: 'lazy', decoding: 'async', draggable: false });
    img.addEventListener('error', mono, { once: true });
    img.src = app.icon_url;
    box.appendChild(img);
  } else {
    mono();
  }
  return box;
}
function initial(name) {
  const ch = Array.from(name.trim())[0] || '?';
  return ch.toUpperCase();
}
function hash(s) {
  let h = 2166136261;
  for (let i = 0; i < s.length; i++) h = Math.imul(h ^ s.charCodeAt(i), 16777619);
  return h >>> 0;
}

// ------------------------------------------------------------------ actions

async function openApp(app, newWindow = false, cardEl = null) {
  cardEl?.classList.remove('launching');
  void cardEl?.offsetWidth;
  cardEl?.classList.add('launching');
  try {
    await invoke('open_app', { id: app.id, newWindow });
  } catch (e) {
    toast(String(e), true);
  }
}

async function togglePin(app) {
  app.pinned = await invoke('toggle_pin', { id: app.id });
  render();
  focusCard(app.id);
}

function focusCard(id) {
  document.querySelector(`.card[data-id="${CSS.escape(id)}"]`)?.focus();
}

async function copy(text) {
  try {
    await navigator.clipboard.writeText(text);
  } catch {
    const ta = el('textarea', { value: text });
    document.body.appendChild(ta);
    ta.select();
    document.execCommand('copy');
    ta.remove();
  }
  toast(t('copied'));
}

const run = (fn) => async () => {
  try { await fn(); } catch (e) { toast(String(e), true); }
};

function menuItems(app) {
  const isRunning = running.has(app.id);
  const s = overview.settings;
  return [
    { label: t('open'), icon: ICONS.open, shortcut: 'Enter', action: () => openApp(app) },
    { label: t('openNewWindow'), icon: ICONS.window, shortcut: 'Ctrl+Click', action: () => openApp(app, true) },
    { label: app.pinned ? t('unpin') : t('pin'), icon: ICONS.pin, action: () => togglePin(app) },
    isRunning && '-',
    isRunning && { label: t('reload'), icon: ICONS.refresh, action: run(() => invoke('reload_app', { id: app.id })) },
    isRunning && s.devtools && { label: t('devtools'), icon: ICONS.code, action: run(() => invoke('open_devtools', { id: app.id })) },
    isRunning && { label: t('closeApp'), icon: ICONS.close, action: run(() => invoke('close_app', { id: app.id })) },
    '-',
    { label: t('revealFolder'), icon: ICONS.folder, action: run(() => invoke('open_location', { kind: 'app', id: app.id })) },
    s.isolation === 'profile' && overview.info.storage_measurable &&
      { label: t('openDataDir'), icon: ICONS.database, action: run(() => invoke('open_location', { kind: 'profile', id: app.id })) },
    overview.info.shortcuts_supported && { label: t('createShortcut'), icon: ICONS.shortcut, action: run(async () => {
      toast(t('shortcutCreated', await invoke('create_shortcut', { id: app.id })));
    }) },
    { label: t('copyCommand'), icon: ICONS.terminal, action: run(async () => copy(await invoke('launch_command', { id: app.id }))) },
    { label: t('details'), icon: ICONS.info, action: () => showDetails(app) },
    s.isolation === 'profile' && '-',
    s.isolation === 'profile' && { label: t('clearData'), icon: ICONS.trash, danger: true, action: () => clearData(app) },
  ].filter(Boolean);
}

async function clearData(app) {
  const ok = await confirmDialog(t('clearTitle', app.name), t('clearText'), t('clearOk'));
  if (!ok) return;
  try {
    await invoke('clear_app_data', { id: app.id });
    toast(t('cleared'));
  } catch (e) {
    toast(String(e), true);
  }
}

// ------------------------------------------------------------------ context menu

const menu = $('#menu');
let menuReturnFocus = null;

function showMenu(app, x, y) {
  menuReturnFocus = document.activeElement;
  const items = menuItems(app).filter((it, i, arr) => it !== '-' || (i > 0 && arr[i - 1] !== '-' && i < arr.length - 1));
  menu.replaceChildren(...items.map((it) => {
    if (it === '-') return el('hr');
    const b = el('button', { type: 'button' });
    b.setAttribute('role', 'menuitem');
    if (it.danger) b.classList.add('danger');
    b.innerHTML = it.icon;
    b.append(document.createTextNode(it.label));
    if (it.shortcut) b.appendChild(el('span', { className: 'shortcut', textContent: it.shortcut }));
    b.addEventListener('click', () => { hideMenu(); it.action(); });
    return b;
  }));
  menu.hidden = false;
  const { innerWidth: w, innerHeight: h } = window;
  const r = menu.getBoundingClientRect();
  menu.style.left = Math.max(8, Math.min(x, w - r.width - 8)) + 'px';
  menu.style.top = Math.max(8, Math.min(y, h - r.height - 8)) + 'px';
  menu.querySelector('button')?.focus();
}
function hideMenu() {
  if (menu.hidden) return;
  menu.hidden = true;
  menuReturnFocus?.focus?.();
}
menu.addEventListener('keydown', (e) => {
  const buttons = [...menu.querySelectorAll('button')];
  const i = buttons.indexOf(document.activeElement);
  if (e.key === 'ArrowDown') { e.preventDefault(); buttons[(i + 1) % buttons.length].focus(); }
  if (e.key === 'ArrowUp') { e.preventDefault(); buttons[(i - 1 + buttons.length) % buttons.length].focus(); }
  if (e.key === 'Home') { e.preventDefault(); buttons[0].focus(); }
  if (e.key === 'End') { e.preventDefault(); buttons[buttons.length - 1].focus(); }
  if (e.key === 'Escape' || e.key === 'Tab') { e.preventDefault(); hideMenu(); }
});
document.addEventListener('mousedown', (e) => { if (!menu.contains(e.target)) hideMenu(); });
window.addEventListener('blur', hideMenu);
window.addEventListener('resize', hideMenu);
$('#content').addEventListener('scroll', hideMenu, { passive: true });

// ------------------------------------------------------------------ dialogs

function formatBytes(n) {
  if (n == null) return '—';
  const units = ['B', 'KB', 'MB', 'GB'];
  let i = 0;
  while (n >= 1024 && i < units.length - 1) { n /= 1024; i++; }
  return `${n.toFixed(i ? 1 : 0)} ${units[i]}`;
}

async function showDetails(app) {
  const dlg = $('#dlg-details');
  $('#details-icon').replaceChildren(iconFor(app));
  $('#details-name').textContent = app.name;
  $('#details-desc').textContent = app.description || '';
  const rows = [
    [t('id'), app.id],
    app.version && [t('version'), app.version],
    app.author && [t('author'), app.author],
    [t('folder'), app.dir],
    app.source.type === 'local' ? [t('entry'), app.source.entry] : [t('url'), app.source.url],
    [t('launches'), String(app.launches)],
    [t('lastOpened'), app.last_opened ? new Date(app.last_opened).toLocaleString() : t('never')],
    [t('storage'), overview.info.storage_measurable ? t('measuring') : t('notMeasurable')],
  ].filter(Boolean);
  const kv = $('#details-kv');
  kv.replaceChildren(...rows.flatMap(([k, v]) => [el('dt', { textContent: k }), el('dd', { textContent: v })]));
  const actions = $('#details-actions');
  const btn = (label, cls, fn) => {
    const b = el('button', { type: 'button', className: 'btn ' + cls, textContent: label });
    b.addEventListener('click', fn);
    return b;
  };
  actions.replaceChildren(
    btn(t('revealFolder'), '', run(() => invoke('open_location', { kind: 'app', id: app.id }))),
    btn(t('open'), 'primary', () => { dlg.close(); openApp(app); }),
  );
  dlg.showModal();
  actions.lastElementChild.focus();
  if (overview.info.storage_measurable) {
    try {
      const size = await invoke('storage_usage', { id: app.id });
      kv.lastElementChild.textContent = size == null ? '—' : formatBytes(size);
    } catch {
      kv.lastElementChild.textContent = '—';
    }
  }
}

function confirmDialog(title, text, okLabel) {
  const dlg = $('#dlg-confirm');
  $('#confirm-title').textContent = title;
  $('#confirm-text').textContent = text;
  $('#confirm-ok').textContent = okLabel;
  dlg.returnValue = '';
  dlg.showModal();
  return new Promise((resolve) => {
    dlg.addEventListener('close', () => resolve(dlg.returnValue === 'ok'), { once: true });
  });
}

function showSettings() {
  const s = overview.settings;
  const info = overview.info;
  const box = $('#settings');
  const group = (title, ...rows) => {
    const g = el('section');
    g.appendChild(el('h4', { textContent: title }));
    g.append(...rows);
    return g;
  };
  const row = (label, hint, control) => {
    const r = el('div', { className: 'setting' });
    const l = el('div', { className: 'label' });
    l.appendChild(el('b', { textContent: label }));
    if (hint instanceof Node) l.appendChild(hint);
    else if (hint) l.appendChild(el('small', { textContent: hint }));
    r.append(l);
    if (control) r.append(control);
    return r;
  };
  const toggle = (key) => {
    const w = el('label', { className: 'switch' });
    const input = el('input', { type: 'checkbox', checked: !!s[key] });
    input.addEventListener('change', async () => {
      try {
        await invoke('set_setting', { key, value: input.checked });
        s[key] = input.checked;
      } catch (e) {
        input.checked = !input.checked;
        toast(String(e), true);
      }
    });
    w.append(input, el('span'));
    return w;
  };
  const select = (key, options, current, onChange) => {
    const sel = el('select', { className: 'select' });
    for (const [value, label] of options) sel.appendChild(el('option', { value, textContent: label }));
    sel.value = current;
    sel.addEventListener('change', onChange || (async () => {
      try {
        await invoke('set_setting', { key, value: sel.value });
        s[key] = sel.value;
      } catch (e) {
        sel.value = current;
        toast(String(e), true);
      }
    }));
    return sel;
  };
  const pathHint = (p) => el('code', { className: 'path', textContent: p });
  const button = (label, fn) => {
    const b = el('button', { type: 'button', className: 'btn small', textContent: label });
    b.addEventListener('click', fn);
    return b;
  };

  box.replaceChildren(
    group(t('sLocations'),
      row(t('sApps'), pathHint(info.apps_dir), button(t('change'), run(async () => {
        const next = await invoke('choose_apps_dir');
        if (next) { overview = next; render(); showSettings(); }
      }))),
      row(t('sConfig'), pathHint(info.config_file), button(t('revealConfig'), run(() => invoke('open_location', { kind: 'config' })))),
      row(t('sData'), pathHint(info.data_dir), button(t('open'), run(() => invoke('open_location', { kind: 'data' })))),
    ),
    group(t('sLauncher'),
      row(t('sAutoOpen'), '', toggle('auto_open_single')),
      row(t('sHideOnLaunch'), '', toggle('hide_on_launch')),
      row(t('sTray'), t('sTrayHint'), toggle('tray')),
      row(t('sKeepInTray'), '', toggle('keep_in_tray')),
      row(t('sWatch'), '', toggle('watch')),
      row(t('sRemember'), '', toggle('remember_state')),
    ),
    group(t('sWebview'),
      row(t('sIsolation'), t('sIsolationHint'), select('isolation', [
        ['profile', t('sIsolationProfile')], ['shared', t('sIsolationShared')],
      ], s.isolation)),
      row(t('sExternal'), '', select('external_links', [
        ['browser', t('sExternalBrowser')], ['allow', t('sExternalAllow')], ['block', t('sExternalBlock')],
      ], s.external_links)),
      row(t('sDevtools'), '', toggle('devtools')),
      row(t('sLiveReload'), '', toggle('live_reload')),
    ),
    group(t('sAppearance'),
      row(t('sTheme'), '', select('theme', [
        ['system', t('themeSystem')], ['light', t('themeLight')], ['dark', t('themeDark')],
      ], prefs.theme, (e) => { prefs.theme = e.target.value; savePrefs(); applyTheme(); })),
    ),
    group(t('sMaintenance'),
      row(t('about', info.version, info.platform), '', el('div', { className: 'toolbar' }, [
        button(t('openLogs'), run(() => invoke('open_location', { kind: 'logs' }))),
        button(t('openDownloads'), run(() => invoke('open_location', { kind: 'downloads' }))),
      ])),
    ),
  );
  const dlg = $('#dlg-settings');
  if (!dlg.open) dlg.showModal();
}

function applyTheme() {
  if (prefs.theme === 'system') delete document.documentElement.dataset.theme;
  else document.documentElement.dataset.theme = prefs.theme;
}

// ------------------------------------------------------------------ toasts

function toast(message, isError = false) {
  const n = el('div', { className: 'toast' + (isError ? ' error' : ''), textContent: message });
  $('#toasts').appendChild(n);
  setTimeout(() => {
    n.classList.add('leaving');
    setTimeout(() => n.remove(), 300);
  }, isError ? 6000 : 3000);
}

// ------------------------------------------------------------------ keyboard

function cards() {
  return [...document.querySelectorAll('.card')];
}
function moveFocus(key) {
  const list = cards();
  if (!list.length) return;
  const i = list.indexOf(document.activeElement);
  if (i < 0) { list[0].focus(); return; }
  if (key === 'ArrowLeft') return list[Math.max(0, i - 1)].focus();
  if (key === 'ArrowRight') return list[Math.min(list.length - 1, i + 1)].focus();
  // Up/down: nearest card in the next/previous visual row.
  const cur = list[i].getBoundingClientRect();
  const dir = key === 'ArrowDown' ? 1 : -1;
  let best = null;
  let bestScore = Infinity;
  for (const c of list) {
    const r = c.getBoundingClientRect();
    const dy = (r.top - cur.top) * dir;
    if (dy <= 4) continue;
    const score = dy * 4 + Math.abs(r.left - cur.left);
    if (score < bestScore) { bestScore = score; best = c; }
  }
  best?.focus();
  best?.scrollIntoView({ block: 'nearest' });
}

document.addEventListener('keydown', (e) => {
  const search = $('#search');
  const inDialog = document.querySelector('dialog[open]');
  if (inDialog || !menu.hidden) return;
  if ((e.key === '/' && document.activeElement !== search) || ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'k')) {
    e.preventDefault();
    search.focus();
    search.select();
    return;
  }
  if (e.key === 'F5' || ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'r')) {
    e.preventDefault();
    rescan();
    return;
  }
  if (document.activeElement === search) {
    if (e.key === 'Escape') { search.value = ''; render(); search.blur(); }
    if (e.key === 'ArrowDown') { e.preventDefault(); cards()[0]?.focus(); }
    if (e.key === 'Enter') {
      const first = cards()[0];
      const app = first && overview.apps.find((a) => a.id === first.dataset.id);
      if (app) openApp(app, e.ctrlKey || e.metaKey, first);
    }
    return;
  }
  if (['ArrowUp', 'ArrowDown', 'ArrowLeft', 'ArrowRight'].includes(e.key)) {
    e.preventDefault();
    moveFocus(e.key);
  } else if (e.key.length === 1 && !e.ctrlKey && !e.metaKey && !e.altKey && e.key !== ' ') {
    // Type-to-search.
    search.focus();
  }
});

async function rescan() {
  const btn = $('#btn-refresh');
  btn.animate([{ transform: 'rotate(0)' }, { transform: 'rotate(360deg)' }], { duration: 500 });
  try {
    overview = await invoke('rescan');
    running = new Set(overview.running);
    render();
  } catch (e) {
    toast(String(e), true);
  }
}

async function importApp() {
  try {
    const id = await invoke('import_app', {});
    if (id != null) toast(t('imported', 1));
  } catch (e) {
    toast(`${t('importFailed')}: ${e}`, true);
  }
}

// ------------------------------------------------------------------ helpers

function el(tag, props = {}, children = []) {
  const n = Object.assign(document.createElement(tag), props);
  if (children.length) n.append(...children);
  return n;
}
function iconButton(label, svg, onClick) {
  const b = el('button', { type: 'button', className: 'icon-btn', title: label });
  b.setAttribute('aria-label', label);
  b.innerHTML = svg;
  b.addEventListener('click', onClick);
  b.addEventListener('keydown', (e) => e.stopPropagation());
  return b;
}

const svg = (d) => `<svg viewBox="0 0 24 24" aria-hidden="true">${d}</svg>`;
const ICONS = {
  pin: svg('<path d="M12 17v5"/><path d="M9 3h6l-1 6 4 4H6l4-4z"/>'),
  more: svg('<circle cx="5" cy="12" r="1.3"/><circle cx="12" cy="12" r="1.3"/><circle cx="19" cy="12" r="1.3"/>'),
  open: svg('<path d="M14 4h6v6"/><path d="M20 4 11 13"/><path d="M19 14v5a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V6a1 1 0 0 1 1-1h5"/>'),
  window: svg('<rect x="3" y="5" width="18" height="15" rx="2"/><path d="M3 9h18"/>'),
  refresh: svg('<path d="M20 11a8 8 0 1 0-2.3 5.7"/><path d="M20 4v7h-7"/>'),
  code: svg('<path d="m8 8-4 4 4 4M16 8l4 4-4 4"/>'),
  close: svg('<path d="M6 6l12 12M18 6 6 18"/>'),
  folder: svg('<path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/>'),
  database: svg('<ellipse cx="12" cy="6" rx="7" ry="3"/><path d="M5 6v12c0 1.7 3.1 3 7 3s7-1.3 7-3V6"/><path d="M5 12c0 1.7 3.1 3 7 3s7-1.3 7-3"/>'),
  shortcut: svg('<rect x="4" y="4" width="16" height="16" rx="3"/><path d="M10 14l5-5M11 9h4v4"/>'),
  terminal: svg('<path d="m5 8 4 4-4 4M12 16h7"/>'),
  info: svg('<circle cx="12" cy="12" r="9"/><path d="M12 11v5M12 8h.01"/>'),
  trash: svg('<path d="M4 7h16M10 11v6M14 11v6M6 7l1 12a2 2 0 0 0 2 2h6a2 2 0 0 0 2-2l1-12M9 7V4h6v3"/>'),
};

// ------------------------------------------------------------------ mock backend (browser preview)

function mockInvoke(cmd, args) {
  const apps = [
    { id: 'notes', name: 'Notes', description: 'Markdown notes stored in IndexedDB', category: 'Productivity', version: '1.2.0', mode: 'protocol', launches: 9, last_opened: Date.now() - 3.6e6, pinned: true },
    { id: 'paint', name: 'Pixel Paint', description: 'Tiny pixel-art editor', category: 'Creative', mode: 'protocol', launches: 2, last_opened: Date.now() - 864e5 * 3 },
    { id: 'docs', name: 'Team Docs', description: 'Opens the web version in its own window', category: 'Productivity', mode: 'remote', launches: 0, last_opened: 0 },
    { id: 'pwa', name: 'Offline PWA', description: 'Uses a service worker, served over http://127.0.0.1', category: 'Productivity', mode: 'localhost', launches: 1, last_opened: Date.now() - 864e5 },
  ].map((a) => Object.assign({ author: '', folder: a.id, dir: '/apps/' + a.id, source: { type: 'local', entry: 'index.html' }, pinned: false, icon_url: null, theme_color: null, version: '' }, a));
  const data = {
    apps, running: ['notes'], notice: null,
    info: { title: 'WebDock', version: 'dev', platform: 'browser', config_file: '/cfg/webdock.toml', apps_dir: '/apps', data_dir: '/data', shortcuts_supported: true, storage_measurable: true },
    settings: { auto_open_single: true, hide_on_launch: false, tray: true, keep_in_tray: false, watch: true, devtools: true, live_reload: false, isolation: 'profile', external_links: 'browser', remember_state: true },
  };
  window.__mock = window.__mock || data;
  const m = window.__mock;
  switch (cmd) {
    case 'get_overview':
    case 'rescan': return Promise.resolve(m);
    case 'toggle_pin': { const a = m.apps.find((x) => x.id === args.id); a.pinned = !a.pinned; return Promise.resolve(a.pinned); }
    case 'storage_usage': return Promise.resolve(1234567);
    case 'launch_command': return Promise.resolve(`webdock --app ${args.id}`);
    default: console.log('mock invoke', cmd, args); return Promise.resolve(null);
  }
}

// ------------------------------------------------------------------ init

function init() {
  applyI18n();
  applyTheme();
  $('#search').addEventListener('input', render);
  $('#sort').addEventListener('change', (e) => { prefs.sort = e.target.value; savePrefs(); render(); });
  $('#view-grid').addEventListener('click', () => { prefs.view = 'grid'; savePrefs(); render(); });
  $('#view-list').addEventListener('click', () => { prefs.view = 'list'; savePrefs(); render(); });
  $('#btn-refresh').addEventListener('click', rescan);
  $('#btn-settings').addEventListener('click', showSettings);
  $('#btn-import').addEventListener('click', importApp);
  $('#empty-import').addEventListener('click', importApp);
  $('#empty-open').addEventListener('click', run(() => invoke('open_location', { kind: 'apps' })));
  $('#status-path').addEventListener('click', run(() => invoke('open_location', { kind: 'apps' })));
  $('#empty-change').addEventListener('click', run(async () => {
    const next = await invoke('choose_apps_dir');
    if (next) { overview = next; render(); }
  }));
  // Keep settings in sync if the config file was edited meanwhile.
  $('#dlg-settings').addEventListener('close', load);
  // Refresh relative "smart" ordering and running state when the window regains focus.
  window.addEventListener('focus', () => { if (overview) load(); });
  // Block the WebView's default context menu outside cards (feels more native).
  document.addEventListener('contextmenu', (e) => {
    if (!e.target.closest('input, textarea, .kv, .path')) e.preventDefault();
  });
  load();
}

init();
