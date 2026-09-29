// Notes stored in IndexedDB; routes like /note/<id> use the History API
// (WebDock's SPA fallback serves index.html for them).
const db = await new Promise((resolve, reject) => {
  const req = indexedDB.open('notes', 1);
  req.onupgradeneeded = () => req.result.createObjectStore('notes', { keyPath: 'id' });
  req.onsuccess = () => resolve(req.result);
  req.onerror = () => reject(req.error);
});
const tx = (mode, fn) => new Promise((resolve, reject) => {
  const t = db.transaction('notes', mode);
  const r = fn(t.objectStore('notes'));
  t.oncomplete = () => resolve(r?.result);
  t.onerror = () => reject(t.error);
});
const $ = (id) => document.getElementById(id);
let notes = await tx('readonly', (s) => s.getAll());
let current = null;
let timer = null;

function route() {
  const m = location.pathname.match(/^\/note\/(.+)$/);
  const id = m && decodeURIComponent(m[1]);
  current = notes.find((n) => n.id === id) || [...notes].sort((a, b) => b.updated - a.updated)[0] || null;
  $('title').value = current?.title ?? '';
  $('body').value = current?.body ?? '';
  $('saved').textContent = current ? 'Saved ' + new Date(current.updated).toLocaleString() : '';
  render();
}
function go(id) { history.pushState(null, '', '/note/' + encodeURIComponent(id)); route(); }

function render() {
  const q = $('q').value.trim().toLowerCase();
  $('list').replaceChildren(...[...notes]
    .filter((n) => !q || (n.title + n.body).toLowerCase().includes(q))
    .sort((a, b) => b.updated - a.updated)
    .map((n) => {
      const li = document.createElement('li');
      li.className = n === current ? 'active' : '';
      const b = document.createElement('b'); b.textContent = n.title || 'Untitled';
      const s = document.createElement('small'); s.textContent = new Date(n.updated).toLocaleDateString();
      li.append(b, s);
      li.onclick = () => go(n.id);
      return li;
    }));
}

async function create() {
  const n = { id: crypto.randomUUID(), title: '', body: '', updated: Date.now() };
  notes.push(n);
  await tx('readwrite', (s) => s.put(n));
  go(n.id);
  $('title').focus();
}

function scheduleSave() {
  if (!current) return;
  current.title = $('title').value;
  current.body = $('body').value;
  current.updated = Date.now();
  render();
  clearTimeout(timer);
  timer = setTimeout(async () => {
    await tx('readwrite', (s) => s.put(current));
    $('saved').textContent = 'Saved ' + new Date(current.updated).toLocaleTimeString();
  }, 300);
}

$('new').onclick = create;
$('title').oninput = scheduleSave;
$('body').oninput = scheduleSave;
$('q').oninput = render;
$('del').onclick = async () => {
  if (!current || !confirm('Delete this note?')) return;
  await tx('readwrite', (s) => s.delete(current.id));
  notes = notes.filter((n) => n !== current);
  history.replaceState(null, '', '/');
  route();
};
addEventListener('popstate', route);
addEventListener('keydown', (e) => { if ((e.ctrlKey || e.metaKey) && e.key === 'n') { e.preventDefault(); create(); } });

if (!notes.length) {
  const welcome = { id: crypto.randomUUID(), title: 'Welcome to Notes', body: 'Notes are stored in this app\'s own IndexedDB,\nso they survive closing the window or restarting WebDock.\n\nCreate a few notes, then try "Clear data…" in the launcher.', updated: Date.now() };
  notes.push(welcome);
  await tx('readwrite', (s) => s.put(welcome));
}
route();
