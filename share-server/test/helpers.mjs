import fs from 'node:fs';
import http from 'node:http';
import net from 'node:net';
import os from 'node:os';
import path from 'node:path';
import { once } from 'node:events';

import { createApp } from '../lib/app.mjs';

export const TOKEN = 'test-token-123';

// Starts a server on a free port with its own temp data dir.
export async function startServer(options = {}) {
  const dataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'cinesnap-share-test-'));
  const clock = { now: Date.now() };
  const app = createApp({
    dataDir,
    uploadToken: TOKEN,
    log: () => {},
    now: () => clock.now,
    ...options,
  });
  app.server.listen(0, '127.0.0.1');
  await once(app.server, 'listening');
  const { port } = app.server.address();
  const base = `http://127.0.0.1:${port}`;

  return {
    base,
    port,
    dataDir,
    clock,
    videoFiles: () => fs.readdirSync(path.join(dataDir, 'videos')),
    async stop() {
      await app.close();
      fs.rmSync(dataDir, { recursive: true, force: true });
    },
  };
}

// Not a playable file, but it has a real ftyp box up front.
export function fakeMp4(size = 4096) {
  const buf = Buffer.alloc(size);
  for (let i = 0; i < size; i++) buf[i] = i % 251;
  buf.writeUInt32BE(24, 0);
  buf.write('ftypisom', 4, 'latin1');
  buf.writeUInt32BE(0x200, 12);
  buf.write('isomiso2', 16, 'latin1');
  return buf;
}

export function fakeGif(width = 320, height = 200) {
  const buf = Buffer.alloc(64);
  buf.write('GIF89a', 0, 'latin1');
  buf.writeUInt16LE(width, 6);
  buf.writeUInt16LE(height, 8);
  buf[63] = 0x3b;
  return buf;
}

export function upload(srv, body, { type = 'video/mp4', token = TOKEN, title, headers = {} } = {}) {
  const h = { ...headers };
  if (type) h['Content-Type'] = type;
  if (token) h.Authorization = `Bearer ${token}`;
  if (title !== undefined) h['X-Title'] = encodeURIComponent(title);
  return fetch(`${srv.base}/api/videos`, { method: 'POST', headers: h, body });
}

// fetch() tidies up paths, so traversal attempts have to go out by hand.
export function rawGet(srv, rawPath, { method = 'GET', headers = {} } = {}) {
  return new Promise((resolve, reject) => {
    const req = http.request(
      { host: '127.0.0.1', port: srv.port, method, path: rawPath, headers },
      (res) => {
        const chunks = [];
        res.on('data', (c) => chunks.push(c));
        res.on('end', () =>
          resolve({ status: res.statusCode, headers: res.headers, body: Buffer.concat(chunks) }),
        );
      },
    );
    req.on('error', reject);
    req.end();
  });
}

// Writes raw bytes to the socket and returns whatever comes back before it closes.
export function rawSocket(srv, payload, { closeAfterMs } = {}) {
  return new Promise((resolve) => {
    const chunks = [];
    const socket = net.connect(srv.port, '127.0.0.1', () => {
      socket.write(payload);
      if (closeAfterMs !== undefined) setTimeout(() => socket.destroy(), closeAfterMs);
    });
    socket.on('data', (c) => chunks.push(c));
    socket.on('error', () => {});
    socket.on('close', () => resolve(Buffer.concat(chunks).toString('latin1')));
  });
}
