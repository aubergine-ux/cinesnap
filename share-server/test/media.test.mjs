import { after, before, describe, test } from 'node:test';
import assert from 'node:assert/strict';

import { parseRange } from '../lib/media.mjs';
import { fakeGif, fakeMp4, startServer, upload } from './helpers.mjs';

describe('media', () => {
  let srv;
  let url;
  const file = fakeMp4(10000);
  const get = (headers = {}, method = 'GET') => fetch(url, { method, headers });

  before(async () => {
    srv = await startServer();
    url = (await (await upload(srv, file)).json()).mediaUrl;
  });
  after(() => srv.stop());

  test('full file', async () => {
    const res = await get();
    assert.equal(res.status, 200);
    assert.equal(res.headers.get('content-type'), 'video/mp4');
    assert.equal(res.headers.get('content-length'), '10000');
    assert.equal(res.headers.get('accept-ranges'), 'bytes');
    assert.equal(res.headers.get('x-content-type-options'), 'nosniff');
    assert.match(res.headers.get('cache-control'), /max-age=31536000/);
    assert.match(res.headers.get('etag'), /^"[0-9a-f]+-[0-9a-f]+"$/);
    assert.ok(!Number.isNaN(Date.parse(res.headers.get('last-modified'))));
    assert.ok(Buffer.from(await res.arrayBuffer()).equals(file));
  });

  test('gif content type', async () => {
    const gif = await (await upload(srv, fakeGif(), { type: 'image/gif' })).json();
    const res = await fetch(gif.mediaUrl);
    assert.equal(res.status, 200);
    assert.equal(res.headers.get('content-type'), 'image/gif');
    assert.ok(Buffer.from(await res.arrayBuffer()).equals(fakeGif()));
  });

  test('head', async () => {
    const res = await get({}, 'HEAD');
    assert.equal(res.status, 200);
    assert.equal(res.headers.get('content-length'), '10000');
    assert.equal((await res.arrayBuffer()).byteLength, 0);
  });

  test('first bytes', async () => {
    const res = await get({ Range: 'bytes=0-99' });
    assert.equal(res.status, 206);
    assert.equal(res.headers.get('content-range'), 'bytes 0-99/10000');
    assert.equal(res.headers.get('content-length'), '100');
    assert.ok(Buffer.from(await res.arrayBuffer()).equals(file.subarray(0, 100)));
  });

  test('middle and open-ended ranges', async () => {
    let res = await get({ Range: 'bytes=500-1499' });
    assert.equal(res.status, 206);
    assert.ok(Buffer.from(await res.arrayBuffer()).equals(file.subarray(500, 1500)));

    res = await get({ Range: 'bytes=9000-' });
    assert.equal(res.status, 206);
    assert.equal(res.headers.get('content-range'), 'bytes 9000-9999/10000');
    assert.ok(Buffer.from(await res.arrayBuffer()).equals(file.subarray(9000)));

    res = await get({ Range: 'bytes=9990-20000' });
    assert.equal(res.status, 206);
    assert.equal(res.headers.get('content-range'), 'bytes 9990-9999/10000');
    assert.equal((await res.arrayBuffer()).byteLength, 10);
  });

  test('suffix range', async () => {
    let res = await get({ Range: 'bytes=-100' });
    assert.equal(res.status, 206);
    assert.equal(res.headers.get('content-range'), 'bytes 9900-9999/10000');
    assert.ok(Buffer.from(await res.arrayBuffer()).equals(file.subarray(9900)));

    res = await get({ Range: 'bytes=-50000' });
    assert.equal(res.status, 206);
    assert.equal(res.headers.get('content-range'), 'bytes 0-9999/10000');
    await res.arrayBuffer();
  });

  test('ranged head', async () => {
    const res = await get({ Range: 'bytes=0-0' }, 'HEAD');
    assert.equal(res.status, 206);
    assert.equal(res.headers.get('content-range'), 'bytes 0-0/10000');
  });

  test('invalid ranges get 416', async () => {
    for (const range of ['bytes=10000-', 'bytes=20000-30000', 'bytes=5-2', 'bytes=abc', 'bytes=-', 'bytes=-0', 'bytes=', 'bytes=1-2-3', 'bytes=99999999999999999999-']) {
      const res = await get({ Range: range });
      assert.equal(res.status, 416, range);
      assert.equal(res.headers.get('content-range'), 'bytes */10000', range);
      await res.arrayBuffer();
    }
  });

  test('ranges the server does not do are answered with the whole file', async () => {
    for (const range of ['items=0-5', 'bytes=0-1,5-6']) {
      const res = await get({ Range: range });
      assert.equal(res.status, 200, range);
      assert.equal((await res.arrayBuffer()).byteLength, 10000);
    }
  });

  test('conditional requests', async () => {
    const first = await get();
    await first.arrayBuffer();
    const etag = first.headers.get('etag');
    const modified = first.headers.get('last-modified');

    let res = await get({ 'If-None-Match': etag });
    assert.equal(res.status, 304);
    res = await get({ 'If-None-Match': '"other"' });
    assert.equal(res.status, 200);
    await res.arrayBuffer();
    res = await get({ 'If-Modified-Since': modified });
    assert.equal(res.status, 304);

    res = await get({ Range: 'bytes=0-9', 'If-Range': etag });
    assert.equal(res.status, 206);
    await res.arrayBuffer();
    res = await get({ Range: 'bytes=0-9', 'If-Range': '"stale"' });
    assert.equal(res.status, 200);
    await res.arrayBuffer();
  });
});

test('parseRange', () => {
  assert.equal(parseRange(undefined, 100), null);
  assert.equal(parseRange('', 100), null);
  assert.deepEqual(parseRange('bytes=0-', 100), { start: 0, end: 99 });
  assert.deepEqual(parseRange('bytes= 10 - 19 ', 100), { start: 10, end: 19 });
  assert.deepEqual(parseRange('BYTES=-1', 100), { start: 99, end: 99 });
  assert.equal(parseRange('bytes=0-', 0), 'unsatisfiable');
  assert.equal(parseRange('bytes=-5', 0), 'unsatisfiable');
  assert.equal(parseRange('bytes=100-', 100), 'unsatisfiable');
});
