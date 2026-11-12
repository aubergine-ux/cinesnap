import fs from 'node:fs';
import { pipeline } from 'node:stream';

// Returns null to send the whole file, 'unsatisfiable' for a 416, or { start, end } (inclusive).
export function parseRange(header, size) {
  if (typeof header !== 'string') return null;
  const m = /^bytes=(.*)$/i.exec(header.trim());
  if (!m) return null;
  const spec = m[1].trim();
  // Several ranges in one request are allowed to be answered with the full file.
  if (spec.includes(',')) return null;

  const parts = /^(\d{0,15})\s*-\s*(\d{0,15})$/.exec(spec);
  if (!parts || (parts[1] === '' && parts[2] === '')) return 'unsatisfiable';

  if (parts[1] === '') {
    const suffix = Number(parts[2]);
    if (suffix === 0 || size === 0) return 'unsatisfiable';
    return { start: Math.max(0, size - suffix), end: size - 1 };
  }

  const start = Number(parts[1]);
  const end = parts[2] === '' ? size - 1 : Math.min(Number(parts[2]), size - 1);
  if (start >= size || end < start) return 'unsatisfiable';
  return { start, end };
}

function notModified(req, etag, mtimeSec) {
  const inm = req.headers['if-none-match'];
  if (inm) {
    return inm.split(',').some((t) => {
      const tag = t.trim().replace(/^W\//, '');
      return tag === '*' || tag === etag;
    });
  }
  const ims = Date.parse(req.headers['if-modified-since'] ?? '');
  return Number.isFinite(ims) && mtimeSec * 1000 <= ims;
}

export function serveFile(req, res, file, contentType) {
  fs.stat(file, (err, st) => {
    if (err || !st.isFile()) {
      res.writeHead(404, { 'Content-Type': 'text/plain; charset=utf-8' });
      res.end('not found\n');
      return;
    }

    const size = st.size;
    const mtimeSec = Math.floor(st.mtimeMs / 1000);
    const lastModified = new Date(mtimeSec * 1000).toUTCString();
    const etag = `"${size.toString(16)}-${mtimeSec.toString(16)}"`;

    const headers = {
      'Content-Type': contentType,
      'Accept-Ranges': 'bytes',
      ETag: etag,
      'Last-Modified': lastModified,
      'Cache-Control': 'public, max-age=31536000, immutable',
    };

    if (notModified(req, etag, mtimeSec)) {
      res.writeHead(304, headers);
      res.end();
      return;
    }

    let range = parseRange(req.headers.range, size);
    const ifRange = req.headers['if-range'];
    if (range && ifRange && ifRange !== etag && ifRange !== lastModified) range = null;

    if (range === 'unsatisfiable') {
      res.writeHead(416, { ...headers, 'Content-Range': `bytes */${size}`, 'Content-Length': 0 });
      res.end();
      return;
    }

    let status = 200;
    let start = 0;
    let end = size - 1;
    if (range) {
      status = 206;
      ({ start, end } = range);
      headers['Content-Range'] = `bytes ${start}-${end}/${size}`;
    }
    headers['Content-Length'] = size === 0 ? 0 : end - start + 1;
    res.writeHead(status, headers);

    if (req.method === 'HEAD' || size === 0) {
      res.end();
      return;
    }
    // A viewer closing the tab mid-stream is normal, nothing to report.
    pipeline(fs.createReadStream(file, { start, end }), res, () => {});
  });
}
