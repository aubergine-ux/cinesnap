import http from 'node:http';
import fs from 'node:fs';
import path from 'node:path';
import { createHash, randomBytes, timingSafeEqual } from 'node:crypto';

import { openDb } from './db.mjs';
import { serveFile } from './media.mjs';
import { TYPES, magicMatches, probeSize } from './probe.mjs';
import { embedPage, esc, landingPage, notFoundPage, playerPage } from './pages.mjs';

const ID_LENGTH = 10;
const ID_ALPHABET = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789';
const ID = `[A-Za-z0-9]{${ID_LENGTH}}`;
const MAX_TITLE = 200;
const EMBED_MAX_WIDTH = 800;

class HttpError extends Error {
  constructor(status, message, headers) {
    super(message);
    this.status = status;
    this.headers = headers;
  }
}

function newId() {
  let id = '';
  while (id.length < ID_LENGTH) {
    for (const b of randomBytes(ID_LENGTH * 2)) {
      // 248 is the largest multiple of 62 under 256, so the modulo stays unbiased
      if (b < 248 && id.length < ID_LENGTH) id += ID_ALPHABET[b % 62];
    }
  }
  return id;
}

function sha256(text) {
  return createHash('sha256').update(text).digest();
}

function firstValue(header) {
  if (Array.isArray(header)) header = header[0];
  return typeof header === 'string' ? header.split(',')[0].trim() : '';
}

function cleanTitle(raw) {
  if (raw === undefined || raw === '') return 'Untitled';
  let title;
  try {
    title = decodeURIComponent(raw);
  } catch {
    throw new HttpError(400, 'X-Title is not valid percent-encoded UTF-8');
  }
  title = title
    .replace(/[\u0000-\u001f\u007f-\u009f]/g, ' ')
    .replace(/\s+/g, ' ')
    .trim();
  return [...title].slice(0, MAX_TITLE).join('') || 'Untitled';
}

function scaled(v, maxWidth, maxHeight = Infinity) {
  const w = v.width || 1280;
  const h = v.height || 720;
  const s = Math.min(1, maxWidth / w, maxHeight / h);
  return { width: Math.max(1, Math.round(w * s)), height: Math.max(1, Math.round(h * s)) };
}

function iframe(src, title, { width, height }) {
  return (
    `<iframe src="${esc(src)}" width="${width}" height="${height}" title="${esc(title)}" ` +
    `style="border:0;max-width:100%;height:auto;aspect-ratio:${width}/${height}" ` +
    `allow="fullscreen; picture-in-picture" allowfullscreen loading="lazy"></iframe>`
  );
}

export function createApp(options = {}) {
  const cfg = {
    port: 8080,
    dataDir: path.resolve('./data'),
    uploadToken: '',
    publicUrl: '',
    maxUploadBytes: 500 * 1024 * 1024,
    trustProxy: 0,
    now: Date.now,
    log: console.log,
    ...options,
  };

  const videoDir = path.join(cfg.dataDir, 'videos');
  fs.mkdirSync(videoDir, { recursive: true });
  // uploads that were cut off by a restart
  for (const name of fs.readdirSync(videoDir)) {
    if (name.startsWith('.upload-')) fs.rmSync(path.join(videoDir, name), { force: true });
  }

  const db = openDb(path.join(cfg.dataDir, 'share.db'));
  const tokenHash = cfg.uploadToken ? sha256(cfg.uploadToken) : null;

  const filePath = (v) => path.join(videoDir, `${v.id}.${TYPES[v.type].ext}`);

  function baseUrl(req) {
    if (cfg.publicUrl) return cfg.publicUrl;
    const fwdProto = firstValue(req.headers['x-forwarded-proto']).toLowerCase();
    const proto = fwdProto === 'https' || fwdProto === 'http' ? fwdProto : 'http';
    let host = firstValue(req.headers['x-forwarded-host']) || firstValue(req.headers.host);
    if (!/^([a-z0-9.-]{1,253}|\[[0-9a-f:.]{2,45}\])(:\d{1,5})?$/i.test(host)) {
      host = `localhost:${cfg.port}`;
    }
    return `${proto}://${host}`;
  }

  function present(v, base) {
    const ext = TYPES[v.type].ext;
    const size = scaled(v, Infinity);
    const embedUrl = `${base}/embed/${v.id}`;
    return {
      id: v.id,
      title: v.title,
      type: v.type,
      bytes: v.bytes,
      width: size.width,
      height: size.height,
      createdAt: new Date(v.created_at).toISOString(),
      views: v.views,
      url: `${base}/v/${v.id}`,
      embedUrl,
      embedHtml: iframe(embedUrl, v.title, scaled(v, EMBED_MAX_WIDTH)),
      mediaUrl: `${base}/media/${v.id}.${ext}`,
    };
  }

  function requireAuth(req) {
    if (!tokenHash) {
      throw new HttpError(503, 'Uploads are disabled: UPLOAD_TOKEN is not set on this server.');
    }
    const m = /^Bearer\s+(.+)$/i.exec(req.headers.authorization ?? '');
    if (!m || !timingSafeEqual(sha256(m[1].trim()), tokenHash)) {
      throw new HttpError(401, 'Missing or wrong upload token.', { 'WWW-Authenticate': 'Bearer' });
    }
  }

  function clientIp(req) {
    if (cfg.trustProxy > 0) {
      const hops = String(req.headers['x-forwarded-for'] ?? '')
        .split(',')
        .map((s) => s.trim())
        .filter(Boolean);
      // each proxy appends the address it saw, so count back from the right
      const ip = hops[Math.max(0, hops.length - cfg.trustProxy)];
      if (ip) return ip;
    }
    return req.socket.remoteAddress ?? '';
  }

  // ---- responses

  const baseHeaders = {
    'X-Content-Type-Options': 'nosniff',
    'Referrer-Policy': 'no-referrer',
  };

  function sendJson(res, status, body, headers = {}) {
    const data = Buffer.from(JSON.stringify(body) + '\n');
    res.writeHead(status, {
      ...baseHeaders,
      'Content-Type': 'application/json; charset=utf-8',
      'Content-Length': data.length,
      'Cache-Control': 'no-store',
      'Content-Security-Policy': "default-src 'none'; frame-ancestors 'none'",
      ...headers,
    });
    res.end(data);
  }

  function sendText(res, status, text) {
    res.writeHead(status, {
      ...baseHeaders,
      'Content-Type': 'text/plain; charset=utf-8',
      'Content-Length': Buffer.byteLength(text),
      'Cache-Control': 'no-store',
    });
    res.end(text);
  }

  // `render` gets the CSP nonce and returns the page. Only the embed page may be framed.
  function sendHtml(res, status, render, { frameable = false } = {}) {
    const nonce = randomBytes(16).toString('base64');
    const data = Buffer.from(render(nonce));
    const csp = [
      "default-src 'none'",
      `style-src 'nonce-${nonce}'`,
      `script-src 'nonce-${nonce}'`,
      "img-src 'self' data:",
      "media-src 'self'",
      "connect-src 'self'",
      "base-uri 'none'",
      "form-action 'none'",
      `frame-ancestors ${frameable ? '*' : "'none'"}`,
    ].join('; ');
    const headers = {
      ...baseHeaders,
      'Content-Type': 'text/html; charset=utf-8',
      'Content-Length': data.length,
      'Cache-Control': 'no-cache',
      'Content-Security-Policy': csp,
    };
    if (!frameable) headers['X-Frame-Options'] = 'DENY';
    res.writeHead(status, headers);
    res.end(data);
  }

  const sendNotFoundPage = (res) => sendHtml(res, 404, (nonce) => notFoundPage({ nonce }));

  // ---- upload

  function receive(req, tmp, type) {
    return new Promise((resolve, reject) => {
      const out = fs.createWriteStream(tmp, { flags: 'wx', mode: 0o640 });
      let bytes = 0;
      let head = Buffer.alloc(0);
      let checked = false;
      let settled = false;

      const fail = (err) => {
        if (settled) return;
        settled = true;
        req.off('data', onData);
        out.destroy();
        fs.rm(tmp, { force: true }, () => reject(err));
      };

      const onData = (chunk) => {
        bytes += chunk.length;
        if (bytes > cfg.maxUploadBytes) return fail(tooLarge());
        if (!checked) {
          head = Buffer.concat([head, chunk.subarray(0, 12 - head.length)]);
          if (head.length >= 12) {
            checked = true;
            if (!magicMatches(type, head)) return fail(badMagic(type));
          }
        }
        if (!out.write(chunk)) {
          req.pause();
          out.once('drain', () => req.resume());
        }
      };

      req.on('data', onData);
      req.on('end', () => {
        if (settled) return;
        if (bytes === 0) return fail(new HttpError(400, 'The request body is empty.'));
        if (!checked) return fail(badMagic(type));
        out.end(() => {
          if (settled) return;
          settled = true;
          resolve(bytes);
        });
      });
      req.on('error', () => fail(new HttpError(400, 'Upload was interrupted.')));
      req.on('close', () => {
        if (!req.complete) fail(new HttpError(400, 'Upload was interrupted.'));
      });
      out.on('error', (err) => fail(err));
    });
  }

  const tooLarge = () =>
    new HttpError(413, `File is too large. The limit is ${Math.floor(cfg.maxUploadBytes / 1024 / 1024)} MB.`);
  const badMagic = (type) => new HttpError(415, `The file contents do not look like ${type}.`);

  async function upload(req, res) {
    requireAuth(req);

    const type = (req.headers['content-type'] ?? '').split(';')[0].trim().toLowerCase();
    if (!Object.hasOwn(TYPES, type)) {
      throw new HttpError(415, 'Content-Type must be video/mp4 or image/gif.');
    }
    const title = cleanTitle(req.headers['x-title']);
    const declared = Number(req.headers['content-length']);
    if (Number.isFinite(declared) && declared > cfg.maxUploadBytes) throw tooLarge();

    if (req.headers.expect) res.writeContinue();

    const tmp = path.join(videoDir, `.upload-${randomBytes(8).toString('hex')}`);
    const bytes = await receive(req, tmp, type);

    const id = newId();
    const video = { id, title, type, bytes, createdAt: cfg.now(), ...probeSize(tmp, type) };
    try {
      fs.renameSync(tmp, filePath(video));
      db.insert(video);
    } catch (err) {
      fs.rmSync(tmp, { force: true });
      fs.rmSync(filePath(video), { force: true });
      throw err;
    }

    cfg.log(`upload ${id} ${type} ${bytes} bytes`);
    const body = present(db.get(id), baseUrl(req));
    sendJson(res, 201, body, { Location: body.url });
  }

  // ---- routes

  function lookup(id) {
    const v = db.get(id);
    if (!v) throw new HttpError(404, 'No such video.');
    return v;
  }

  const routes = [
    ['GET', /^\/healthz$/, (req, res) => sendText(res, 200, 'ok\n')],

    ['GET', /^\/$/, (req, res) => {
      sendHtml(res, 200, (nonce) =>
        landingPage({
          nonce,
          base: baseUrl(req),
          configured: Boolean(tokenHash),
          maxUploadMb: Math.floor(cfg.maxUploadBytes / 1024 / 1024),
        }),
      );
    }],

    ['GET', new RegExp(`^/v/(${ID})$`), (req, res, m) => {
      const row = db.get(m[1]);
      if (!row) return sendNotFoundPage(res);
      const base = baseUrl(req);
      const v = present(row, base);
      const oembedUrl = `${base}/oembed?url=${encodeURIComponent(v.url)}&format=json`;
      sendHtml(res, 200, (nonce) => playerPage(v, { nonce, oembedUrl }));
    }],

    ['GET', new RegExp(`^/embed/(${ID})$`), (req, res, m) => {
      const row = db.get(m[1]);
      if (!row) return sendHtml(res, 404, (nonce) => notFoundPage({ nonce }), { frameable: true });
      const v = present(row, baseUrl(req));
      sendHtml(res, 200, (nonce) => embedPage(v, { nonce }), { frameable: true });
    }],

    ['GET', new RegExp(`^/media/(${ID})\\.(mp4|gif)$`), (req, res, m) => {
      const v = db.get(m[1]);
      if (!v || TYPES[v.type].ext !== m[2]) return sendText(res, 404, 'not found\n');
      res.setHeader('X-Content-Type-Options', 'nosniff');
      serveFile(req, res, filePath(v), v.type);
    }],

    ['GET', /^\/oembed$/, (req, res, m, query) => {
      const format = query.get('format') ?? 'json';
      if (format !== 'json') throw new HttpError(501, 'Only format=json is supported.');
      let target;
      try {
        target = new URL(query.get('url') ?? '');
      } catch {
        throw new HttpError(400, 'The url parameter is missing or not a URL.');
      }
      const match = new RegExp(`^/(?:v|embed)/(${ID})$`).exec(target.pathname);
      const row = match && db.get(match[1]);
      if (!row) throw new HttpError(404, 'No such video.');

      const limit = (name) => {
        const n = Number.parseInt(query.get(name) ?? '', 10);
        return Number.isInteger(n) && n > 0 ? n : Infinity;
      };
      const base = baseUrl(req);
      const size = scaled(row, Math.min(limit('maxwidth'), EMBED_MAX_WIDTH), limit('maxheight'));
      sendJson(
        res,
        200,
        {
          version: '1.0',
          type: 'video',
          title: row.title,
          provider_name: 'Cinesnap',
          provider_url: base,
          html: iframe(`${base}/embed/${row.id}`, row.title, size),
          width: size.width,
          height: size.height,
        },
        { 'Cache-Control': 'public, max-age=300' },
      );
    }],

    ['POST', /^\/api\/videos$/, upload],

    ['GET', /^\/api\/videos$/, (req, res) => {
      requireAuth(req);
      const base = baseUrl(req);
      sendJson(res, 200, db.list().map((v) => present(v, base)));
    }],

    ['GET', new RegExp(`^/api/videos/(${ID})$`), (req, res, m) => {
      sendJson(res, 200, present(lookup(m[1]), baseUrl(req)));
    }],

    ['DELETE', new RegExp(`^/api/videos/(${ID})$`), (req, res, m) => {
      requireAuth(req);
      const v = lookup(m[1]);
      db.remove(v.id);
      fs.rmSync(filePath(v), { force: true });
      cfg.log(`delete ${v.id}`);
      res.writeHead(204, baseHeaders);
      res.end();
    }],

    ['POST', new RegExp(`^/api/videos/(${ID})/view$`), (req, res, m) => {
      const v = lookup(m[1]);
      const visitor = createHash('sha256')
        .update(`${db.salt}\n${clientIp(req)}\n${req.headers['user-agent'] ?? ''}`)
        .digest('hex')
        .slice(0, 32);
      sendJson(res, 200, { views: db.view(v.id, visitor, cfg.now()) });
    }],
  ];

  async function handle(req, res) {
    const raw = req.url ?? '';
    if (!raw.startsWith('/')) throw new HttpError(400, 'Bad request.');
    const cut = raw.indexOf('?');
    const pathname = cut === -1 ? raw : raw.slice(0, cut);
    const query = new URLSearchParams(cut === -1 ? '' : raw.slice(cut + 1));
    const method = req.method === 'HEAD' ? 'GET' : req.method;

    const allowed = [];
    for (const [verb, pattern, handler] of routes) {
      const m = pattern.exec(pathname);
      if (!m) continue;
      if (verb === method) return handler(req, res, m, query);
      allowed.push(verb);
    }
    if (allowed.length) {
      throw new HttpError(405, 'Method not allowed.', { Allow: allowed.join(', ') });
    }
    if (pathname.startsWith('/api/') || pathname === '/oembed') throw new HttpError(404, 'Not found.');
    if (pathname.startsWith('/media/')) return sendText(res, 404, 'not found\n');
    if (method !== 'GET') throw new HttpError(404, 'Not found.');
    sendNotFoundPage(res);
  }

  function onRequest(req, res) {
    handle(req, res).catch((err) => {
      const status = err instanceof HttpError ? err.status : 500;
      if (status === 500) cfg.log(`error ${req.method} ${req.url}: ${err?.stack ?? err}`);
      if (res.headersSent || res.destroyed) {
        res.destroy();
        return;
      }
      sendJson(res, status, { error: status === 500 ? 'Internal server error.' : err.message }, err.headers);

      // A rejected upload may still be sending. Let it finish for a while, then hang up.
      if (!req.complete) {
        req.resume();
        const timer = setTimeout(() => req.socket.destroy(), 30_000);
        timer.unref();
        req.once('end', () => clearTimeout(timer));
        req.socket.once('close', () => clearTimeout(timer));
      }
    });
  }

  const server = http.createServer(onRequest);
  // Without this node answers "Expect: 100-continue" before the token and size are looked at.
  server.on('checkContinue', onRequest);
  server.on('clientError', (err, socket) => {
    if (socket.writable && !socket.destroyed) {
      socket.end('HTTP/1.1 400 Bad Request\r\nConnection: close\r\nContent-Length: 0\r\n\r\n');
    } else {
      socket.destroy();
    }
  });

  server.headersTimeout = 20_000;
  server.requestTimeout = 60 * 60 * 1000;
  server.keepAliveTimeout = 10_000;
  server.setTimeout(120_000);

  const pruneTimer = setInterval(() => db.prune(cfg.now()), 10 * 60 * 1000);
  pruneTimer.unref();

  let closed = false;
  function close() {
    if (closed) return Promise.resolve();
    closed = true;
    clearInterval(pruneTimer);
    return new Promise((resolve) => {
      server.close(() => {
        db.close();
        resolve();
      });
      server.closeAllConnections();
    });
  }

  return { server, close, config: cfg };
}
