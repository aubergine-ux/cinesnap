import fs from 'node:fs';

export const TYPES = {
  'video/mp4': { ext: 'mp4', kind: 'video' },
  'image/gif': { ext: 'gif', kind: 'image' },
};

// `head` is the first bytes of the upload (12 or more).
export function magicMatches(type, head) {
  if (type === 'video/mp4') {
    return head.length >= 8 && head.toString('latin1', 4, 8) === 'ftyp';
  }
  if (type === 'image/gif') {
    const sig = head.toString('latin1', 0, 6);
    return sig === 'GIF87a' || sig === 'GIF89a';
  }
  return false;
}

// Best effort pixel size. Returns null when it cannot be worked out.
export function probeSize(file, type) {
  let fd;
  try {
    fd = fs.openSync(file, 'r');
    const size = type === 'image/gif' ? gifSize(fd) : mp4Size(fd);
    if (size && size.width > 0 && size.height > 0) return size;
  } catch {
    // unreadable or odd file, the player copes without a size
  } finally {
    if (fd !== undefined) fs.closeSync(fd);
  }
  return null;
}

function gifSize(fd) {
  const buf = Buffer.alloc(10);
  if (fs.readSync(fd, buf, 0, 10, 0) < 10) return null;
  return { width: buf.readUInt16LE(6), height: buf.readUInt16LE(8) };
}

const MAX_MOOV = 64 * 1024 * 1024;

function mp4Size(fd) {
  const fileSize = fs.fstatSync(fd).size;
  const header = Buffer.alloc(16);
  let pos = 0;

  // Walk the top level boxes until moov shows up (it can sit after mdat).
  while (pos + 8 <= fileSize) {
    if (fs.readSync(fd, header, 0, 16, pos) < 8) return null;
    let size = header.readUInt32BE(0);
    const name = header.toString('latin1', 4, 8);
    let headerLen = 8;
    if (size === 1) {
      size = Number(header.readBigUInt64BE(8));
      headerLen = 16;
    } else if (size === 0) {
      size = fileSize - pos;
    }
    if (size < headerLen) return null;

    if (name === 'moov') {
      const len = Math.min(size, fileSize - pos) - headerLen;
      if (len <= 0 || len > MAX_MOOV) return null;
      const moov = Buffer.alloc(len);
      fs.readSync(fd, moov, 0, len, pos + headerLen);
      return trackSize(moov);
    }
    pos += size;
  }
  return null;
}

function* boxes(buf) {
  let pos = 0;
  while (pos + 8 <= buf.length) {
    const size = buf.readUInt32BE(pos);
    if (size < 8 || pos + size > buf.length) return;
    yield { name: buf.toString('latin1', pos + 4, pos + 8), body: buf.subarray(pos + 8, pos + size) };
    pos += size;
  }
}

function trackSize(moov) {
  for (const trak of boxes(moov)) {
    if (trak.name !== 'trak') continue;
    for (const box of boxes(trak.body)) {
      if (box.name !== 'tkhd' || box.body.length < 8) continue;
      // width and height are the last two fields, 16.16 fixed point
      const width = box.body.readUInt32BE(box.body.length - 8) >>> 16;
      const height = box.body.readUInt32BE(box.body.length - 4) >>> 16;
      if (width > 0 && height > 0) return { width, height };
    }
  }
  return null;
}
