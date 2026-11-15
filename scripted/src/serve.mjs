// The app under recording: either a command we start and wait for, or a
// directory served by a small built-in static server.

import { spawn, spawnSync } from 'node:child_process';
import fs from 'node:fs';
import http from 'node:http';
import path from 'node:path';

const TYPES = {
  '.html': 'text/html; charset=utf-8',
  '.htm': 'text/html; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
  '.mjs': 'text/javascript; charset=utf-8',
  '.css': 'text/css; charset=utf-8',
  '.json': 'application/json',
  '.svg': 'image/svg+xml',
  '.png': 'image/png',
  '.jpg': 'image/jpeg',
  '.jpeg': 'image/jpeg',
  '.gif': 'image/gif',
  '.webp': 'image/webp',
  '.avif': 'image/avif',
  '.ico': 'image/x-icon',
  '.woff': 'font/woff',
  '.woff2': 'font/woff2',
  '.ttf': 'font/ttf',
  '.wasm': 'application/wasm',
  '.txt': 'text/plain; charset=utf-8',
  '.map': 'application/json',
  '.mp4': 'video/mp4',
  '.webm': 'video/webm',
};

// Serve `root` on 127.0.0.1. Resolves with { url, stop }.
export function serveDir(root, port = 0) {
  if (!fs.existsSync(root) || !fs.statSync(root).isDirectory()) {
    return Promise.reject(new Error(`serve: ${root} is not a directory`));
  }
  const server = http.createServer((req, res) => {
    let rel;
    try {
      rel = decodeURIComponent(new URL(req.url, 'http://x').pathname);
    } catch {
      res.writeHead(400).end();
      return;
    }
    let file = path.join(root, path.normalize(rel));
    if (file !== root && !file.startsWith(root + path.sep)) {
      res.writeHead(403).end();
      return;
    }
    fs.stat(file, (err, st) => {
      if (!err && st.isDirectory()) file = path.join(file, 'index.html');
      fs.readFile(file, (err2, data) => {
        if (err2) {
          res.writeHead(404, { 'content-type': 'text/plain' }).end('not found');
          return;
        }
        res.writeHead(200, {
          'content-type': TYPES[path.extname(file).toLowerCase()] ?? 'application/octet-stream',
          'cache-control': 'no-store',
        });
        res.end(data);
      });
    });
  });
  return new Promise((resolve, reject) => {
    server.once('error', (e) => reject(new Error(`serve: cannot listen on port ${port}: ${e.message}`)));
    server.listen(port, '127.0.0.1', () => {
      resolve({
        url: `http://127.0.0.1:${server.address().port}`,
        stop: () =>
          new Promise((done) => {
            server.closeAllConnections?.();
            server.close(() => done());
          }),
      });
    });
  });
}

async function responds(url) {
  try {
    const res = await fetch(url, { signal: AbortSignal.timeout(2000), redirect: 'manual' });
    // Any answer short of a server error means something is listening.
    return res.status < 500;
  } catch {
    return false;
  }
}

function killTree(child) {
  if (child.exitCode !== null || child.signalCode !== null) return;
  if (process.platform === 'win32') {
    spawnSync('taskkill', ['/pid', String(child.pid), '/T', '/F'], { stdio: 'ignore' });
    return;
  }
  // The child leads its own process group, so this takes its children too.
  for (const sig of ['SIGTERM', 'SIGKILL']) {
    try {
      process.kill(-child.pid, sig);
    } catch {
      return;
    }
    if (sig === 'SIGTERM') {
      const until = Date.now() + 3000;
      while (Date.now() < until) {
        try {
          process.kill(-child.pid, 0);
        } catch {
          return;
        }
        Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 50);
      }
    }
  }
}

// Start `command` through the shell and wait until `url` answers.
// Resolves with { url, stop }.
export async function serveCommand({ command, url, cwd, timeout }, log = () => {}) {
  if (await responds(url)) {
    throw new Error(`serve: ${url} already responds before "${command}" was started; stop the other server first`);
  }
  const child = spawn(command, {
    cwd,
    shell: true,
    detached: process.platform !== 'win32',
    stdio: ['ignore', 'pipe', 'pipe'],
    windowsHide: true,
  });
  let tail = '';
  const keep = (d) => {
    tail = (tail + d).slice(-4000);
  };
  child.stdout.on('data', keep);
  child.stderr.on('data', keep);
  let failed = null;
  child.on('error', (e) => (failed = `cannot start "${command}": ${e.message}`));
  child.on('exit', (code, signal) => (failed ??= `"${command}" exited early (${signal ?? `code ${code}`})`));

  const stop = () => killTree(child);
  const deadline = Date.now() + timeout * 1000;
  log(`starting: ${command}`);
  for (;;) {
    if (failed) {
      stop();
      throw new Error(`serve: ${failed}${tail ? `\n${tail.trimEnd()}` : ''}`);
    }
    if (await responds(url)) return { url, stop };
    if (Date.now() > deadline) {
      stop();
      throw new Error(`serve: ${url} did not respond within ${timeout} s${tail ? `\n${tail.trimEnd()}` : ''}`);
    }
    await new Promise((r) => setTimeout(r, 250));
  }
}
