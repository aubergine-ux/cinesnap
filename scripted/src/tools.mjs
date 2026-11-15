// Finding and running the external programs: ffmpeg and the Cinesnap CLI.

import { spawn } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';

function isFile(p) {
  try {
    return fs.statSync(p).isFile();
  } catch {
    return false;
  }
}

export function onPath(name) {
  const exts = process.platform === 'win32' ? ['.exe', '.cmd', ''] : [''];
  for (const dir of (process.env.PATH ?? '').split(path.delimiter)) {
    if (!dir) continue;
    for (const ext of exts) {
      const p = path.join(dir, name + ext);
      if (isFile(p)) return p;
    }
  }
  return null;
}

// An explicit path (flag, then environment) must exist; otherwise look on PATH.
function find(flag, envName, names, what) {
  const explicit = flag ?? (process.env[envName] || undefined);
  if (explicit) {
    if (isFile(explicit)) return path.resolve(explicit);
    const found = !/[\\/]/.test(explicit) && onPath(explicit);
    if (found) return found;
    throw new Error(`${what} not found at ${explicit}`);
  }
  for (const n of names) {
    const p = onPath(n);
    if (p) return p;
  }
  return null;
}

export function findFfmpeg(flag) {
  const p = find(flag, 'CINESNAP_FFMPEG', ['ffmpeg'], 'ffmpeg');
  if (!p) throw new Error('ffmpeg not found; install it, or pass --ffmpeg or set CINESNAP_FFMPEG');
  return p;
}

// Null when it is not installed; callers decide whether that matters.
export function findCinesnap(flag) {
  return find(flag, 'CINESNAP_BIN', ['cinesnap-cli', 'cinesnap'], 'cinesnap');
}

// Run a program to completion. Resolves with { code, stdout, stderr }.
export function run(cmd, args, { env, inherit = false, input } = {}) {
  return new Promise((resolve, reject) => {
    const child = spawn(cmd, args, {
      env: env ? { ...process.env, ...env } : process.env,
      stdio: [input === undefined ? 'ignore' : 'pipe', inherit ? 'inherit' : 'pipe', inherit ? 'inherit' : 'pipe'],
      windowsHide: true,
    });
    let stdout = '';
    let stderr = '';
    child.stdout?.on('data', (d) => (stdout += d));
    child.stderr?.on('data', (d) => (stderr += d));
    child.on('error', (e) => reject(new Error(`cannot run ${cmd}: ${e.message}`)));
    child.on('close', (code, signal) => resolve({ code: code ?? (signal ? 1 : 0), stdout, stderr }));
    if (input !== undefined) child.stdin.end(input);
  });
}
