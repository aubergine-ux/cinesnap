// Screen capture through the DevTools screencast: every frame the page
// presents arrives as a PNG with the time it was shown, which is what lets
// the event log line up with the video.

import { spawn } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { run } from './tools.mjs';

export class Capture {
  constructor(cdp, dir, size) {
    this.cdp = cdp;
    this.dir = dir;
    this.size = size; // largest frame we want, in device pixels
    this.frames = []; // { ts, file } in arrival order, ts in epoch ms
    this.t0 = null;
    this.writes = [];
    this.error = null;
    this.onFirst = null;
    this.onFrame = (f) => {
      // Not acknowledging a frame stalls the screencast.
      cdp.send('Page.screencastFrameAck', { sessionId: f.sessionId }).catch(() => {});
      const now = Date.now();
      let ts = (f.metadata.timestamp ?? 0) * 1000;
      // The stamp is wall-clock time. If it is missing or clearly on another
      // clock, the arrival time is the next best thing.
      if (!(now - ts > -50 && now - ts < 500)) ts = now - 4;
      const last = this.frames.at(-1);
      if (last && ts <= last.ts) ts = last.ts + 0.01;
      const file = path.join(dir, `${String(this.frames.length).padStart(6, '0')}.png`);
      this.frames.push({ ts, file });
      this.writes.push(
        fs.promises.writeFile(file, Buffer.from(f.data, 'base64')).catch((e) => {
          this.error ??= e;
        }),
      );
      if (this.onFirst) {
        this.onFirst();
        this.onFirst = null;
      }
    };
  }

  // Milliseconds since the first frame.
  now() {
    return Date.now() - this.t0;
  }

  // Start capturing. Time zero is the first frame's own timestamp.
  async start() {
    this.cdp.on('Page.screencastFrame', this.onFrame);
    const first = new Promise((resolve, reject) => {
      this.onFirst = resolve;
      setTimeout(() => reject(new Error('the browser produced no video frames')), 10000).unref();
    });
    await this.cdp.send('Page.startScreencast', {
      format: 'png',
      everyNthFrame: 1,
      maxWidth: this.size.width,
      maxHeight: this.size.height,
    });
    await first;
    this.t0 = this.frames[0].ts;
  }

  // Stop and return the length of the recording in ms.
  async stop() {
    const end = this.now();
    // Let the frame for the last moment arrive before closing the tap.
    await new Promise((r) => setTimeout(r, 80));
    await this.cdp.send('Page.stopScreencast').catch(() => {});
    this.cdp.off('Page.screencastFrame', this.onFrame);
    await Promise.all(this.writes);
    if (this.error) throw new Error(`could not store captured frames: ${this.error.message}`);
    return end;
  }
}

// Width and height from a PNG header.
export function pngSize(buf) {
  if (buf.length < 24 || buf.readUInt32BE(0) !== 0x89504e47) throw new Error('captured frame is not a PNG');
  return { width: buf.readUInt32BE(16), height: buf.readUInt32BE(20) };
}

// Which captured frame each output frame shows: the newest one presented by
// the middle of the output frame's interval. `stamps` are ms from time zero.
export function pickFrames(stamps, durationMs, fps) {
  const count = Math.max(1, Math.ceil((durationMs * fps) / 1000 - 1e-6));
  const out = new Array(count);
  let j = 0;
  for (let i = 0; i < count; i++) {
    const at = ((i + 0.5) * 1000) / fps;
    while (j + 1 < stamps.length && stamps[j + 1] <= at) j++;
    out[i] = j;
  }
  return out;
}

// Turn the captured frames into a constant frame rate H.264 file.
// Resolves with { width, height, frames, durationMs } of the result.
export async function encode(capture, durationMs, fps, ffmpeg, out) {
  const { frames, t0 } = capture;
  const picks = pickFrames(
    frames.map((f) => f.ts - t0),
    durationMs,
    fps,
  );
  const first = pngSize(await fs.promises.readFile(frames[0].file));
  const width = first.width - (first.width % 2);
  const height = first.height - (first.height % 2);

  const child = spawn(
    ffmpeg,
    [
      '-v', 'error', '-y',
      '-f', 'image2pipe', '-framerate', String(fps), '-c:v', 'png', '-i', '-',
      '-vf', `crop=${width}:${height}:0:0`,
      '-c:v', 'libx264', '-preset', 'medium', '-crf', '14', '-pix_fmt', 'yuv420p',
      '-r', String(fps), '-movflags', '+faststart', '-an',
      out,
    ],
    { stdio: ['pipe', 'ignore', 'pipe'], windowsHide: true },
  );
  let stderr = '';
  child.stderr.on('data', (d) => (stderr += d));
  const done = new Promise((resolve, reject) => {
    child.on('error', (e) => reject(new Error(`cannot run ffmpeg: ${e.message}`)));
    child.on('close', (code) => resolve(code));
  });
  child.stdin.on('error', () => {}); // ffmpeg died; the exit code tells the story

  let loaded = -1;
  let buf = null;
  for (const j of picks) {
    if (j !== loaded) {
      buf = await fs.promises.readFile(frames[j].file);
      loaded = j;
    }
    if (child.exitCode !== null) break;
    if (!child.stdin.write(buf)) await Promise.race([new Promise((r) => child.stdin.once('drain', r)), done]);
  }
  child.stdin.end();
  const code = await done;
  if (code !== 0) throw new Error(`ffmpeg failed to encode the video\n${stderr.trim()}`);

  const count = await countFrames(ffmpeg, out);
  return { width, height, frames: count, durationMs: (count * 1000) / fps };
}

// Frames actually in the file, by demuxing it.
export async function countFrames(ffmpeg, file) {
  const r = await run(ffmpeg, ['-v', 'error', '-stats', '-i', file, '-map', '0:v:0', '-c', 'copy', '-f', 'null', '-']);
  const all = [...r.stderr.matchAll(/frame=\s*(\d+)/g)];
  if (r.code !== 0 || all.length === 0) throw new Error(`could not read back ${file}\n${r.stderr.trim()}`);
  return Number(all.at(-1)[1]);
}
