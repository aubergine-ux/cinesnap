import { after, before, describe, test } from 'node:test';
import assert from 'node:assert/strict';

import { TOKEN, fakeGif, fakeMp4, rawGet, rawSocket, startServer, upload } from './helpers.mjs';

describe('upload', () => {
  let srv;
  before(async () => {
    srv = await startServer({ maxUploadBytes: 1024 * 1024 });
  });
  after(() => srv.stop());

  test('mp4 upload returns links', async () => {
    const res = await upload(srv, fakeMp4(), { title: 'My demo' });
    assert.equal(res.status, 201);
    const body = await res.json();
    assert.match(body.id, /^[A-Za-z0-9]{10}$/);
    assert.equal(body.url, `${srv.base}/v/${body.id}`);
    assert.equal(body.embedUrl, `${srv.base}/embed/${body.id}`);
    assert.equal(body.mediaUrl, `${srv.base}/media/${body.id}.mp4`);
    assert.match(body.embedHtml, /^<iframe src="[^"]+\/embed\/[A-Za-z0-9]{10}" .*<\/iframe>$/);
    assert.equal(res.headers.get('location'), body.url);
    assert.deepEqual(srv.videoFiles().filter((f) => f.startsWith(body.id)), [`${body.id}.mp4`]);
  });

  test('gif upload', async () => {
    const res = await upload(srv, fakeGif(), { type: 'image/gif' });
    assert.equal(res.status, 201);
    const body = await res.json();
    assert.ok(body.mediaUrl.endsWith(`${body.id}.gif`));
  });

  test('content type parameters are ignored', async () => {
    const res = await upload(srv, fakeMp4(), { type: 'video/mp4; codecs="avc1"' });
    assert.equal(res.status, 201);
  });

  test('no token', async () => {
    const before = srv.videoFiles().length;
    const res = await upload(srv, fakeMp4(), { token: null });
    assert.equal(res.status, 401);
    assert.equal(res.headers.get('www-authenticate'), 'Bearer');
    assert.ok((await res.json()).error);
    assert.equal(srv.videoFiles().length, before);
  });

  test('wrong token', async () => {
    for (const token of ['nope', TOKEN + 'x', TOKEN.slice(0, -1)]) {
      const res = await upload(srv, fakeMp4(), { token });
      assert.equal(res.status, 401, token);
    }
  });

  test('wrong content type', async () => {
    for (const type of ['video/webm', 'text/html', 'application/octet-stream', null]) {
      const res = await upload(srv, fakeMp4(), { type });
      assert.equal(res.status, 415, String(type));
    }
  });

  test('bad magic bytes', async () => {
    const before = srv.videoFiles().length;
    const notVideo = Buffer.from('<html><script>alert(1)</script></html>'.repeat(10));
    assert.equal((await upload(srv, notVideo)).status, 415);
    assert.equal((await upload(srv, notVideo, { type: 'image/gif' })).status, 415);
    assert.equal((await upload(srv, fakeGif())).status, 415);
    assert.equal((await upload(srv, fakeMp4(), { type: 'image/gif' })).status, 415);
    assert.equal((await upload(srv, Buffer.from('GIF8'), { type: 'image/gif' })).status, 415);
    assert.equal(srv.videoFiles().length, before);
  });

  test('empty body', async () => {
    const res = await upload(srv, Buffer.alloc(0));
    assert.equal(res.status, 400);
  });

  test('too large, declared up front', async () => {
    const before = srv.videoFiles().length;
    const res = await upload(srv, fakeMp4(1024 * 1024 + 1));
    assert.equal(res.status, 413);
    assert.match((await res.json()).error, /1 MB/);
    assert.equal(srv.videoFiles().length, before);
  });

  test('too large, streamed without a length', async () => {
    const before = srv.videoFiles().length;
    const chunk = fakeMp4(256 * 1024);
    const body = new ReadableStream({
      start(controller) {
        for (let i = 0; i < 6; i++) controller.enqueue(chunk);
        controller.close();
      },
    });
    const res = await fetch(`${srv.base}/api/videos`, {
      method: 'POST',
      headers: { 'Content-Type': 'video/mp4', Authorization: `Bearer ${TOKEN}` },
      body,
      duplex: 'half',
    });
    assert.equal(res.status, 413);
    await res.arrayBuffer();
    assert.equal(srv.videoFiles().length, before);
  });

  test('expect: 100-continue is refused before the body when the token is wrong', async () => {
    const reply = await rawSocket(
      srv,
      'POST /api/videos HTTP/1.1\r\nHost: x\r\nContent-Type: video/mp4\r\nContent-Length: 500000\r\n' +
        'Authorization: Bearer wrong\r\nExpect: 100-continue\r\nConnection: close\r\n\r\n',
      { closeAfterMs: 300 },
    );
    assert.match(reply, /^HTTP\/1\.1 401 /);
    assert.doesNotMatch(reply, /100 Continue/);
  });

  test('bad percent-encoding in the title', async () => {
    const res = await upload(srv, fakeMp4(), { headers: { 'X-Title': '%E0%A4%A' } });
    assert.equal(res.status, 400);
  });

  test('title is decoded, trimmed and capped', async () => {
    const res = await upload(srv, fakeMp4(), { title: `  Démo ✓\n${'x'.repeat(400)}` });
    const { id } = await res.json();
    const meta = await (await fetch(`${srv.base}/api/videos/${id}`)).json();
    assert.ok(meta.title.startsWith('Démo ✓ xxx'));
    assert.equal([...meta.title].length, 200);
  });

  test('aborted upload leaves nothing behind', async () => {
    const before = srv.videoFiles();
    await rawSocket(
      srv,
      Buffer.concat([
        Buffer.from(
          `POST /api/videos HTTP/1.1\r\nHost: x\r\nContent-Type: video/mp4\r\nContent-Length: 100000\r\nAuthorization: Bearer ${TOKEN}\r\n\r\n`,
        ),
        fakeMp4(2000),
      ]),
      { closeAfterMs: 150 },
    );
    await new Promise((r) => setTimeout(r, 150));
    assert.deepEqual(srv.videoFiles(), before);
    assert.equal((await fetch(`${srv.base}/healthz`)).status, 200);
  });

  test('garbage on the socket gets a 400 and the server lives on', async () => {
    const reply = await rawSocket(srv, 'THIS IS NOT HTTP\r\n\r\n');
    assert.match(reply, /^HTTP\/1\.1 400 /);
    assert.equal(await (await fetch(`${srv.base}/healthz`)).text(), 'ok\n');
  });
});

describe('uploads without a configured token', () => {
  let srv;
  before(async () => {
    srv = await startServer({ uploadToken: '' });
  });
  after(() => srv.stop());

  test('upload is refused with 503', async () => {
    for (const token of [null, '', 'anything']) {
      const res = await upload(srv, fakeMp4(), { token });
      assert.equal(res.status, 503);
      assert.match((await res.json()).error, /UPLOAD_TOKEN/);
    }
    assert.deepEqual(srv.videoFiles(), []);
  });

  test('list and delete are refused too', async () => {
    assert.equal((await fetch(`${srv.base}/api/videos`)).status, 503);
    const res = await fetch(`${srv.base}/api/videos/abcdefghij`, { method: 'DELETE' });
    assert.equal(res.status, 503);
  });

  test('landing page explains the setup', async () => {
    const html = await (await fetch(`${srv.base}/`)).text();
    assert.match(html, /UPLOAD_TOKEN/);
    assert.match(html, /openssl rand -hex 24/);
  });
});

describe('metadata, list, delete', () => {
  let srv;
  const auth = { Authorization: `Bearer ${TOKEN}` };
  before(async () => {
    srv = await startServer();
  });
  after(() => srv.stop());

  test('metadata', async () => {
    srv.clock.now = Date.UTC(2026, 0, 15, 12, 0, 0);
    const up = await (await upload(srv, fakeMp4(5000), { title: 'Quarterly demo' })).json();
    const res = await fetch(`${srv.base}/api/videos/${up.id}`);
    assert.equal(res.status, 200);
    assert.match(res.headers.get('content-type'), /^application\/json/);
    const meta = await res.json();
    assert.deepEqual(
      { ...meta, width: undefined, height: undefined },
      {
        id: up.id,
        title: 'Quarterly demo',
        type: 'video/mp4',
        bytes: 5000,
        width: undefined,
        height: undefined,
        createdAt: '2026-01-15T12:00:00.000Z',
        views: 0,
        url: up.url,
        embedUrl: up.embedUrl,
        embedHtml: up.embedHtml,
        mediaUrl: up.mediaUrl,
      },
    );
  });

  test('gif size is read from the header', async () => {
    const up = await (await upload(srv, fakeGif(320, 200), { type: 'image/gif' })).json();
    const meta = await (await fetch(`${srv.base}/api/videos/${up.id}`)).json();
    assert.equal(meta.type, 'image/gif');
    assert.equal(meta.title, 'Untitled');
    assert.equal(meta.width, 320);
    assert.equal(meta.height, 200);
    assert.match(meta.embedHtml, /width="320" height="200"/);
  });

  test('unknown id is a json 404', async () => {
    const res = await fetch(`${srv.base}/api/videos/zzzzzzzzzz`);
    assert.equal(res.status, 404);
    assert.ok((await res.json()).error);
  });

  test('list needs the token and is newest first', async () => {
    assert.equal((await fetch(`${srv.base}/api/videos`)).status, 401);
    assert.equal(
      (await fetch(`${srv.base}/api/videos`, { headers: { Authorization: 'Bearer wrong' } })).status,
      401,
    );

    srv.clock.now = Date.UTC(2026, 1, 1);
    const older = await (await upload(srv, fakeMp4(), { title: 'older' })).json();
    srv.clock.now = Date.UTC(2026, 1, 2);
    const newer = await (await upload(srv, fakeMp4(), { title: 'newer' })).json();

    const list = await (await fetch(`${srv.base}/api/videos`, { headers: auth })).json();
    assert.ok(Array.isArray(list));
    assert.equal(list[0].id, newer.id);
    assert.equal(list[1].id, older.id);
    assert.equal(list[0].url, newer.url);
  });

  test('delete removes the row and the file', async () => {
    const up = await (await upload(srv, fakeMp4())).json();
    assert.ok(srv.videoFiles().includes(`${up.id}.mp4`));
    await fetch(`${srv.base}/api/videos/${up.id}/view`, { method: 'POST' });

    assert.equal((await fetch(`${srv.base}/api/videos/${up.id}`, { method: 'DELETE' })).status, 401);
    assert.ok(srv.videoFiles().includes(`${up.id}.mp4`));

    const res = await fetch(`${srv.base}/api/videos/${up.id}`, { method: 'DELETE', headers: auth });
    assert.equal(res.status, 204);
    assert.equal(await res.text(), '');
    assert.ok(!srv.videoFiles().includes(`${up.id}.mp4`));

    assert.equal((await fetch(`${srv.base}/api/videos/${up.id}`)).status, 404);
    assert.equal((await fetch(up.mediaUrl)).status, 404);
    assert.equal((await fetch(up.url)).status, 404);
    assert.equal(
      (await fetch(`${srv.base}/api/videos/${up.id}`, { method: 'DELETE', headers: auth })).status,
      404,
    );
  });

  test('wrong method is a 405', async () => {
    const res = await fetch(`${srv.base}/api/videos`, { method: 'PUT', headers: auth, body: 'x' });
    assert.equal(res.status, 405);
    assert.match(res.headers.get('allow'), /POST/);
  });
});

describe('id validation', () => {
  let srv;
  let id;
  before(async () => {
    srv = await startServer();
    id = (await (await upload(srv, fakeMp4())).json()).id;
  });
  after(() => srv.stop());

  test('traversal attempts never reach the disk', async () => {
    const paths = [
      '/media/../share.db',
      '/media/../../share.db',
      '/media/..%2f..%2fshare.db',
      '/media/%2e%2e/%2e%2e/share.db',
      '/media/....//share.db',
      '/media/share.db',
      `/media/${id}.mp4/../../share.db`,
      `/media/${id}.mp4%00.gif`,
      `/media/${id}`,
      `/media/${id}.gif`,
      `/media/${id}.mp4x`,
      `/media/x${id}.mp4`,
      '/media/.mp4',
      '/media//etc/passwd',
      '/v/../../etc/passwd',
      '/v/..%2f..%2fetc%2fpasswd',
      `/v/${id}/..`,
      `/v/${id}%2f`,
      '/v/short',
      `/v/${id}a`,
      `/embed/${id.slice(0, 9)}.`,
      '/api/videos/..%2F..%2Fshare.db',
      '/api/videos/../../share.db',
      `/api/videos/${id}%2e`,
      '//etc/passwd',
    ];
    for (const p of paths) {
      const res = await rawGet(srv, p);
      assert.equal(res.status, 404, p);
      assert.ok(!res.body.includes('SQLite format'), p);
    }
  });

  test('delete with a crafted id touches nothing', async () => {
    const before = srv.videoFiles();
    for (const p of ['/api/videos/..%2Fshare.db', '/api/videos/../share.db', `/api/videos/${id}.mp4`]) {
      const res = await rawGet(srv, p, { method: 'DELETE', headers: { Authorization: `Bearer ${TOKEN}` } });
      assert.equal(res.status, 404, p);
    }
    assert.deepEqual(srv.videoFiles(), before);
    assert.equal((await fetch(`${srv.base}/api/videos/${id}`)).status, 200);
  });

  test('request targets that are not paths are rejected', async () => {
    const reply = await rawSocket(srv, 'GET http://evil.example/v/x HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n');
    assert.match(reply, /^HTTP\/1\.1 400 /);
  });
});

describe('views', () => {
  let srv;
  let id;
  const view = (headers = {}) =>
    fetch(`${srv.base}/api/videos/${id}/view`, { method: 'POST', headers }).then((r) => r.json());

  before(async () => {
    srv = await startServer({ trustProxy: 1 });
    id = (await (await upload(srv, fakeMp4())).json()).id;
  });
  after(() => srv.stop());

  test('counts once per visitor per hour', async () => {
    const alice = { 'User-Agent': 'alice', 'X-Forwarded-For': '203.0.113.5' };
    assert.deepEqual(await view(alice), { views: 1 });
    assert.deepEqual(await view(alice), { views: 1 });

    // other browser, same address
    assert.deepEqual(await view({ ...alice, 'User-Agent': 'bob' }), { views: 2 });
    // same browser, other address
    assert.deepEqual(await view({ ...alice, 'X-Forwarded-For': '203.0.113.6' }), { views: 3 });
    // a spoofed entry on the left does not make a new visitor
    assert.deepEqual(await view({ ...alice, 'X-Forwarded-For': '1.2.3.4, 203.0.113.5' }), { views: 3 });

    srv.clock.now += 59 * 60 * 1000;
    assert.deepEqual(await view(alice), { views: 3 });
    srv.clock.now += 2 * 60 * 1000;
    assert.deepEqual(await view(alice), { views: 4 });
    assert.deepEqual(await view(alice), { views: 4 });

    const meta = await (await fetch(`${srv.base}/api/videos/${id}`)).json();
    assert.equal(meta.views, 4);
  });

  test('views are per video', async () => {
    const other = (await (await upload(srv, fakeMp4())).json()).id;
    const res = await fetch(`${srv.base}/api/videos/${other}/view`, {
      method: 'POST',
      headers: { 'User-Agent': 'alice', 'X-Forwarded-For': '203.0.113.5' },
    });
    assert.deepEqual(await res.json(), { views: 1 });
  });

  test('unknown video and wrong method', async () => {
    const res = await fetch(`${srv.base}/api/videos/zzzzzzzzzz/view`, { method: 'POST' });
    assert.equal(res.status, 404);
    assert.equal((await fetch(`${srv.base}/api/videos/${id}/view`)).status, 405);
  });

  test('no raw addresses in the database', async () => {
    const fs = await import('node:fs');
    const path = await import('node:path');
    for (const name of fs.readdirSync(srv.dataDir)) {
      const file = path.join(srv.dataDir, name);
      if (!fs.statSync(file).isFile()) continue;
      const text = fs.readFileSync(file, 'latin1');
      assert.ok(!text.includes('203.0.113.'), name);
    }
  });
});
