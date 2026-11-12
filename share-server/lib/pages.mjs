const PROJECT_URL = 'https://github.com/aubergine-ux/cinesnap';

export function esc(value) {
  return String(value).replace(/[&<>"']/g, (c) => `&#${c.charCodeAt(0)};`);
}

export function formatViews(n) {
  return `${Number(n).toLocaleString('en-US')} ${n === 1 ? 'view' : 'views'}`;
}

function formatDate(iso) {
  return new Date(iso).toLocaleDateString('en-US', {
    year: 'numeric',
    month: 'short',
    day: 'numeric',
    timeZone: 'UTC',
  });
}

const ICON =
  "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 32 32'%3E%3Crect width='32' height='32' rx='8' fill='%237c6cff'/%3E%3Cpath d='M13 10.5v11l9-5.5z' fill='%23fff'/%3E%3C/svg%3E";

const icons = {
  link: '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M10 13a5 5 0 0 0 7.07 0l3-3a5 5 0 0 0-7.07-7.07l-1.5 1.5"/><path d="M14 11a5 5 0 0 0-7.07 0l-3 3a5 5 0 0 0 7.07 7.07l1.5-1.5"/></svg>',
  code: '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="m16 18 6-6-6-6"/><path d="m8 6-6 6 6 6"/></svg>',
  download:
    '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 3v12"/><path d="m7 10 5 5 5-5"/><path d="M5 21h14"/></svg>',
};

const BASE_CSS = `
:root {
  color-scheme: light dark;
  --bg: #f5f5f8;
  --surface: #ffffff;
  --text: #15161c;
  --muted: #636777;
  --border: #e0e1e8;
  --accent: #5b4be0;
  --accent-text: #ffffff;
  --accent-soft: rgba(91, 75, 224, 0.1);
  --danger: #c0392b;
  --shadow: 0 1px 2px rgba(20, 20, 40, 0.06), 0 12px 32px -8px rgba(20, 20, 40, 0.18);
}
@media (prefers-color-scheme: dark) {
  :root {
    --bg: #0c0d11;
    --surface: #17181f;
    --text: #eceef4;
    --muted: #979bab;
    --border: #2a2d38;
    --accent: #8f82ff;
    --accent-text: #0c0d11;
    --accent-soft: rgba(124, 108, 255, 0.18);
    --danger: #ff7b6b;
    --shadow: 0 1px 2px rgba(0, 0, 0, 0.4), 0 16px 40px -8px rgba(0, 0, 0, 0.6);
  }
}
* { box-sizing: border-box; }
html { -webkit-text-size-adjust: 100%; }
body {
  margin: 0;
  min-height: 100vh;
  min-height: 100dvh;
  display: flex;
  flex-direction: column;
  background: var(--bg);
  color: var(--text);
  font: 16px/1.5 system-ui, -apple-system, "Segoe UI", Roboto, "Helvetica Neue", Arial, sans-serif;
}
a { color: var(--accent); }
.page { flex: 1; width: 100%; padding: clamp(16px, 4vw, 40px) clamp(14px, 4vw, 40px) 24px; }
footer { padding: 20px 16px 28px; text-align: center; font-size: 13px; color: var(--muted); }
footer a { color: inherit; text-decoration: none; }
footer a:hover { color: var(--text); text-decoration: underline; }
h1 { margin: 0; font-size: clamp(1.25rem, 1rem + 1.2vw, 1.6rem); line-height: 1.25; font-weight: 650; letter-spacing: -0.01em; overflow-wrap: anywhere; }
.btn {
  display: inline-flex; align-items: center; gap: 8px;
  height: 38px; padding: 0 14px;
  border: 1px solid var(--border); border-radius: 10px;
  background: var(--surface); color: var(--text);
  font: inherit; font-size: 14px; font-weight: 550; text-decoration: none; white-space: nowrap;
  cursor: pointer;
  transition: border-color 0.12s, background 0.12s, transform 0.05s;
}
.btn:hover { border-color: var(--accent); }
.btn:active { transform: translateY(1px); }
.btn:focus-visible, input:focus-visible, textarea:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
.btn.primary { background: var(--accent); border-color: var(--accent); color: var(--accent-text); }
.btn.primary:hover { filter: brightness(1.08); }
.btn.danger:hover { border-color: var(--danger); color: var(--danger); }
.btn svg { width: 16px; height: 16px; fill: none; stroke: currentColor; stroke-width: 2; stroke-linecap: round; stroke-linejoin: round; }
code, textarea.code { font: 13px/1.5 ui-monospace, SFMono-Regular, Menlo, Consolas, "Liberation Mono", monospace; }
[hidden] { display: none !important; }
`;

function layout({ title, head = '', css = '', body, script = '', nonce, bodyAttrs = '' }) {
  return `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>${esc(title)}</title>
<link rel="icon" href="${ICON}">
${head}
<style nonce="${nonce}">${BASE_CSS}${css}</style>
</head>
<body${bodyAttrs}>
${body}
${script ? `<script nonce="${nonce}">${script}</script>` : ''}
</body>
</html>
`;
}

const FOOTER = `<footer><a href="${PROJECT_URL}" target="_blank" rel="noopener">Made with Cinesnap</a></footer>`;

// Counts a view once playback really starts. GIFs count as soon as they load.
const VIEW_JS = `
var media = document.getElementById('media');
var viewsEl = document.getElementById('views');
var counted = false;
function countView() {
  if (counted) return;
  counted = true;
  fetch('/api/videos/' + document.body.dataset.id + '/view', { method: 'POST' })
    .then(function (r) { return r.ok ? r.json() : null; })
    .then(function (j) {
      if (j && viewsEl) viewsEl.textContent = j.views.toLocaleString('en-US') + (j.views === 1 ? ' view' : ' views');
    })
    .catch(function () {});
}
if (media.tagName === 'VIDEO') media.addEventListener('playing', countView, { once: true });
else if (media.complete && media.naturalWidth) countView();
else media.addEventListener('load', countView, { once: true });
`;

const COPY_JS = `
function copyText(text) {
  if (navigator.clipboard && window.isSecureContext) {
    return navigator.clipboard.writeText(text).catch(function () { return legacyCopy(text); });
  }
  return legacyCopy(text);
}
function legacyCopy(text) {
  return new Promise(function (resolve, reject) {
    var ta = document.createElement('textarea');
    ta.value = text;
    ta.setAttribute('readonly', '');
    ta.className = 'offscreen';
    document.body.appendChild(ta);
    ta.select();
    var ok = false;
    try { ok = document.execCommand('copy'); } catch (e) {}
    ta.remove();
    ok ? resolve() : reject(new Error('copy failed'));
  });
}
function flash(btn, text) {
  var label = btn.querySelector('span') || btn;
  var old = label.textContent;
  label.textContent = text;
  clearTimeout(btn._t);
  btn._t = setTimeout(function () { label.textContent = btn.dataset.label || old; }, 1600);
  if (!btn.dataset.label) btn.dataset.label = old;
}
`;

function metaTags(v) {
  const isVideo = v.type.startsWith('video/');
  const tags = [
    ['name', 'robots', 'noindex'],
    ['name', 'description', 'Recorded with Cinesnap'],
    ['property', 'og:site_name', 'Cinesnap'],
    ['property', 'og:title', v.title],
    ['property', 'og:description', 'Recorded with Cinesnap'],
    ['property', 'og:url', v.url],
    ['name', 'twitter:title', v.title],
  ];
  if (isVideo) {
    tags.push(
      ['property', 'og:type', 'video.other'],
      ['property', 'og:video', v.mediaUrl],
      ['property', 'og:video:type', v.type],
      ['property', 'og:video:width', v.width],
      ['property', 'og:video:height', v.height],
      ['name', 'twitter:card', 'player'],
      ['name', 'twitter:player', v.embedUrl],
      ['name', 'twitter:player:width', v.width],
      ['name', 'twitter:player:height', v.height],
      ['name', 'twitter:player:stream', v.mediaUrl],
      ['name', 'twitter:player:stream:content_type', v.type],
    );
    if (v.mediaUrl.startsWith('https:')) tags.push(['property', 'og:video:secure_url', v.mediaUrl]);
  } else {
    tags.push(
      ['property', 'og:type', 'website'],
      ['property', 'og:image', v.mediaUrl],
      ['property', 'og:image:type', v.type],
      ['property', 'og:image:width', v.width],
      ['property', 'og:image:height', v.height],
      ['name', 'twitter:card', 'summary_large_image'],
      ['name', 'twitter:image', v.mediaUrl],
    );
  }
  return tags.map(([attr, key, value]) => `<meta ${attr}="${key}" content="${esc(value)}">`).join('\n');
}

function mediaTag(v, src) {
  if (v.type.startsWith('video/')) {
    // The time fragment makes Safari paint the first frame instead of a blank box.
    return `<video id="media" src="${esc(src)}#t=0.001" controls playsinline preload="metadata"></video>`;
  }
  return `<img id="media" src="${esc(src)}" alt="${esc(v.title)}">`;
}

// `v` is the public JSON shape of a video (see present() in app.mjs).
export function playerPage(v, { nonce, oembedUrl }) {
  const src = new URL(v.mediaUrl).pathname;
  const ext = src.split('.').pop();
  const isVideo = v.type.startsWith('video/');
  const fileName =
    (v.title.replace(/[^\p{L}\p{N}._ -]+/gu, ' ').replace(/\s+/g, ' ').trim().slice(0, 80) || `cinesnap-${v.id}`) +
    `.${ext}`;

  const head = `<link rel="canonical" href="${esc(v.url)}">
<link rel="alternate" type="application/json+oembed" href="${esc(oembedUrl)}" title="${esc(v.title)}">
${metaTags(v)}`;

  const css = `
:root { --fit: min(100%, ${Math.max(v.width, 480)}px, calc((100vh - 17rem) * ${v.width} / ${v.height})); }
.wrap { width: 100%; max-width: max(min(100%, 20rem), var(--fit)); margin: 0 auto; }
.stage {
  margin: 0 auto; width: 100%; max-width: max(min(100%, 15rem), var(--fit));
  aspect-ratio: ${v.width} / ${v.height};
  border-radius: 14px; overflow: hidden;
  background: ${isVideo ? '#000' : 'var(--surface)'};
  box-shadow: var(--shadow);
  outline: 1px solid var(--border); outline-offset: -1px;
}
.stage video, .stage img { display: block; width: 100%; height: 100%; object-fit: contain; }
.info { margin-top: clamp(18px, 3vw, 26px); }
.meta { margin: 6px 0 0; color: var(--muted); font-size: 14px; }
.meta .dot { margin: 0 6px; opacity: 0.6; }
.actions { display: flex; flex-wrap: wrap; gap: 8px; margin-top: 18px; }
.embed { margin-top: 12px; padding: 12px; border: 1px solid var(--border); border-radius: 12px; background: var(--surface); }
.embed label { display: block; margin-bottom: 8px; font-size: 13px; color: var(--muted); }
.embed textarea {
  display: block; width: 100%; min-height: 84px; resize: vertical; padding: 10px 12px;
  border: 1px solid var(--border); border-radius: 8px; background: var(--bg); color: var(--text);
  word-break: break-all;
}
.embed .btn { margin-top: 10px; }
.offscreen { position: fixed; top: 0; left: -9999px; opacity: 0; }
@media (max-width: 480px) {
  .stage { border-radius: 10px; }
  .actions .btn { flex: 1 1 auto; justify-content: center; }
}
`;

  const body = `<main class="page">
<div class="wrap">
<figure class="stage">${mediaTag(v, src)}</figure>
<section class="info">
<h1>${esc(v.title)}</h1>
<p class="meta"><span id="views">${formatViews(v.views)}</span><span class="dot">&middot;</span><time id="date" datetime="${esc(v.createdAt)}">${formatDate(v.createdAt)}</time></p>
<div class="actions">
<button class="btn primary" id="copy-link" type="button">${icons.link}<span>Copy link</span></button>
<button class="btn" id="toggle-embed" type="button" aria-expanded="false" aria-controls="embed">${icons.code}<span>Embed</span></button>
<a class="btn" href="${esc(src)}" download="${esc(fileName)}">${icons.download}<span>Download</span></a>
</div>
<div class="embed" id="embed" hidden>
<label for="embed-code">Paste this where you want the player to appear</label>
<textarea class="code" id="embed-code" readonly spellcheck="false" rows="3">${esc(v.embedHtml)}</textarea>
<button class="btn" id="copy-embed" type="button"><span>Copy embed code</span></button>
</div>
</section>
</div>
</main>
${FOOTER}`;

  const script = `${VIEW_JS}${COPY_JS}
var d = document.body.dataset;
var dateEl = document.getElementById('date');
try {
  dateEl.textContent = new Date(dateEl.dateTime).toLocaleDateString(undefined, { year: 'numeric', month: 'short', day: 'numeric' });
} catch (e) {}
document.getElementById('copy-link').addEventListener('click', function () {
  var btn = this;
  copyText(d.url).then(function () { flash(btn, 'Copied'); }, function () { flash(btn, 'Copy failed'); });
});
var embed = document.getElementById('embed');
var code = document.getElementById('embed-code');
document.getElementById('toggle-embed').addEventListener('click', function () {
  embed.hidden = !embed.hidden;
  this.setAttribute('aria-expanded', String(!embed.hidden));
  if (!embed.hidden) { code.focus(); code.select(); }
});
document.getElementById('copy-embed').addEventListener('click', function () {
  var btn = this;
  copyText(code.value).then(function () { flash(btn, 'Copied'); }, function () { flash(btn, 'Copy failed'); });
});
`;

  return layout({
    title: `${v.title} · Cinesnap`,
    head,
    css,
    body,
    script,
    nonce,
    bodyAttrs: ` data-id="${esc(v.id)}" data-url="${esc(v.url)}"`,
  });
}

export function embedPage(v, { nonce }) {
  const src = new URL(v.mediaUrl).pathname;
  const css = `
html, body { height: 100%; }
body { display: block; min-height: 0; background: #000; overflow: hidden; }
#media { display: block; width: 100%; height: 100%; object-fit: contain; background: #000; }
`;
  return layout({
    title: v.title,
    head: `<meta name="robots" content="noindex">\n<link rel="canonical" href="${esc(v.url)}">`,
    css,
    body: mediaTag(v, src),
    script: VIEW_JS,
    nonce,
    bodyAttrs: ` data-id="${esc(v.id)}"`,
  });
}

export function notFoundPage({ nonce }) {
  const css = `
.page { display: grid; place-items: center; }
.lost { max-width: 26rem; text-align: center; }
.lost .mark {
  width: 64px; height: 64px; margin: 0 auto 20px; border-radius: 18px;
  display: grid; place-items: center; background: var(--accent-soft); color: var(--accent);
}
.lost .mark svg { width: 30px; height: 30px; fill: none; stroke: currentColor; stroke-width: 2; stroke-linecap: round; stroke-linejoin: round; }
.lost p { margin: 10px 0 0; color: var(--muted); }
`;
  const body = `<main class="page">
<div class="lost">
<div class="mark"><svg viewBox="0 0 24 24" aria-hidden="true"><rect x="3" y="5" width="18" height="14" rx="3"/><path d="m9.5 9.5 5 5"/><path d="m14.5 9.5-5 5"/></svg></div>
<h1>Nothing to play here</h1>
<p>This link doesn't point to a video. It may have been mistyped, or the recording was deleted by its owner.</p>
</div>
</main>
${FOOTER}`;
  return layout({ title: 'Not found · Cinesnap', head: '<meta name="robots" content="noindex">', css, body, nonce });
}

export function landingPage({ nonce, base, configured, maxUploadMb }) {
  const css = `
.wrap { width: 100%; max-width: 44rem; margin: 0 auto; }
.lead { margin: 10px 0 0; color: var(--muted); font-size: 1.05rem; }
.card { margin-top: 24px; padding: 18px 20px; border: 1px solid var(--border); border-radius: 14px; background: var(--surface); }
.card.warn { border-color: var(--accent); background: var(--accent-soft); }
.card h2 { margin: 0 0 8px; font-size: 1rem; }
.card p { margin: 8px 0 0; }
.card ol { margin: 8px 0 0; padding-left: 1.25rem; }
.card li { margin-top: 4px; }
pre { margin: 10px 0 0; padding: 12px 14px; overflow-x: auto; border-radius: 10px; background: var(--bg); border: 1px solid var(--border); }
.card.warn pre { background: var(--surface); }
.hint { color: var(--muted); font-size: 14px; }
form { display: flex; gap: 8px; margin-top: 12px; }
input[type=password] {
  flex: 1; min-width: 0; height: 38px; padding: 0 12px;
  border: 1px solid var(--border); border-radius: 10px; background: var(--bg); color: var(--text); font: inherit; font-size: 14px;
}
#status { margin-top: 10px; }
#status:empty { display: none; }
#status.error { color: var(--danger); }
.bar { display: flex; align-items: center; justify-content: space-between; gap: 12px; margin-top: 4px; }
.list { list-style: none; margin: 12px 0 0; padding: 0; }
.list li { display: flex; align-items: center; gap: 12px; padding: 12px 0; border-top: 1px solid var(--border); }
.list .what { flex: 1; min-width: 0; }
.list .what a { display: block; font-weight: 600; text-decoration: none; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.list .what a:hover { text-decoration: underline; }
.list .what span { display: block; color: var(--muted); font-size: 13px; }
.list .btn { height: 32px; padding: 0 10px; font-size: 13px; }
.offscreen { position: fixed; top: 0; left: -9999px; opacity: 0; }
@media (max-width: 520px) {
  .list li { flex-wrap: wrap; }
  .list .what { flex-basis: 100%; }
}
`;

  const setup = configured
    ? `<section class="card">
<h2>Uploading</h2>
<p>From the Cinesnap app, add this server and your upload token under sharing. From a terminal:</p>
<pre><code>cinesnap share demo.mp4 --server ${esc(base)} --token &lt;token&gt;</code></pre>
<p class="hint">MP4 and GIF, up to ${esc(maxUploadMb)} MB per file.</p>
</section>`
    : `<section class="card warn">
<h2>Uploads are switched off</h2>
<p>This server has no upload token yet, so it refuses every upload. To finish the setup:</p>
<ol>
<li>Generate a token: <code>openssl rand -hex 24</code></li>
<li>Put it in <code>.env</code> as <code>UPLOAD_TOKEN=&hellip;</code></li>
<li>Restart: <code>docker compose up -d</code></li>
</ol>
</section>`;

  const body = `<main class="page">
<div class="wrap">
<h1>Cinesnap Share</h1>
<p class="lead">A small self-hosted home for screen recordings made with Cinesnap. Upload an MP4 or GIF and get a link with a clean player page, a view count and an embed code.</p>
${setup}
<section class="card"${configured ? '' : ' hidden'}>
<div class="bar">
<h2>Your uploads</h2>
<button class="btn" id="forget" type="button" hidden>Lock</button>
</div>
<form id="unlock">
<input type="password" id="token" placeholder="Upload token" autocomplete="off" aria-label="Upload token" required>
<button class="btn primary" type="submit">Show</button>
</form>
<div class="hint" id="status" role="status"></div>
<ul class="list" id="list"></ul>
</section>
</div>
</main>
${FOOTER}`;

  const script = `${COPY_JS}
var KEY = 'cinesnap-share-token';
var form = document.getElementById('unlock');
var input = document.getElementById('token');
var statusEl = document.getElementById('status');
var list = document.getElementById('list');
var forget = document.getElementById('forget');
var token = '';
try { token = sessionStorage.getItem(KEY) || ''; } catch (e) {}

function say(text, isError) {
  statusEl.textContent = text;
  statusEl.className = isError ? 'hint error' : 'hint';
}
function size(n) {
  if (n < 1024 * 1024) return Math.max(1, Math.round(n / 1024)) + ' KB';
  return (n / 1024 / 1024).toFixed(1) + ' MB';
}
function el(tag, cls, text) {
  var e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text != null) e.textContent = text;
  return e;
}
function lock() {
  token = '';
  try { sessionStorage.removeItem(KEY); } catch (e) {}
  list.textContent = '';
  form.hidden = false;
  forget.hidden = true;
}
function api(method, path) {
  return fetch(path, { method: method, headers: { Authorization: 'Bearer ' + token } });
}
function row(v) {
  var li = el('li');
  var what = el('div', 'what');
  var a = el('a', '', v.title);
  a.href = '/v/' + v.id;
  var when = new Date(v.createdAt).toLocaleDateString(undefined, { year: 'numeric', month: 'short', day: 'numeric' });
  var views = v.views.toLocaleString('en-US') + (v.views === 1 ? ' view' : ' views');
  what.append(a, el('span', '', [views, when, size(v.bytes), v.type === 'image/gif' ? 'GIF' : 'MP4'].join(' \\u00b7 ')));
  var copy = el('button', 'btn', 'Copy link');
  copy.type = 'button';
  copy.addEventListener('click', function () {
    copyText(v.url).then(function () { flash(copy, 'Copied'); }, function () { flash(copy, 'Copy failed'); });
  });
  var del = el('button', 'btn danger', 'Delete');
  del.type = 'button';
  del.addEventListener('click', function () {
    if (!confirm('Delete "' + v.title + '"? The link will stop working.')) return;
    api('DELETE', '/api/videos/' + v.id).then(function (r) {
      if (r.ok || r.status === 404) { li.remove(); if (!list.children.length) say('No uploads yet.'); }
      else say('Could not delete that one.', true);
    }, function () { say('Could not reach the server.', true); });
  });
  li.append(what, copy, del);
  return li;
}
function load() {
  say('Loading\\u2026');
  api('GET', '/api/videos').then(function (r) {
    if (r.status === 401) { lock(); say('That token was not accepted.', true); return null; }
    if (!r.ok) { say('The server answered ' + r.status + '.', true); return null; }
    return r.json();
  }).then(function (videos) {
    if (!videos) return;
    try { sessionStorage.setItem(KEY, token); } catch (e) {}
    form.hidden = true;
    forget.hidden = false;
    list.textContent = '';
    videos.forEach(function (v) { list.appendChild(row(v)); });
    say(videos.length ? '' : 'No uploads yet.');
  }).catch(function () { say('Could not reach the server.', true); });
}
form.addEventListener('submit', function (ev) {
  ev.preventDefault();
  token = input.value.trim();
  input.value = '';
  if (token) load();
});
forget.addEventListener('click', function () { lock(); say(''); });
if (token) load();
`;

  return layout({ title: 'Cinesnap Share', css, body, script, nonce });
}
