import { after, before, describe, test } from 'node:test';
import assert from 'node:assert/strict';

import { fakeGif, fakeMp4, startServer, upload } from './helpers.mjs';

const NASTY = `<script>alert("x")</script> & 'quotes' "><img src=x onerror=alert(1)>`;

describe('pages', () => {
  let srv;
  let video;
  let nasty;
  let gif;

  before(async () => {
    srv = await startServer();
    video = await (await upload(srv, fakeMp4(), { title: 'Launch walkthrough' })).json();
    nasty = await (await upload(srv, fakeMp4(), { title: NASTY })).json();
    gif = await (await upload(srv, fakeGif(400, 300), { type: 'image/gif', title: 'A loop' })).json();
  });
  after(() => srv.stop());

  test('healthz', async () => {
    const res = await fetch(`${srv.base}/healthz`);
    assert.equal(res.status, 200);
    assert.equal((await res.text()).trim(), 'ok');
  });

  test('landing page', async () => {
    const res = await fetch(`${srv.base}/`);
    assert.equal(res.status, 200);
    assert.match(res.headers.get('content-type'), /^text\/html/);
    const html = await res.text();
    assert.match(html, /Cinesnap Share/);
    assert.doesNotMatch(html, /Uploads are switched off/);
  });

  test('player page', async () => {
    const res = await fetch(video.url);
    assert.equal(res.status, 200);
    const html = await res.text();
    assert.match(html, /<h1>Launch walkthrough<\/h1>/);
    assert.match(html, /0 views/);
    assert.ok(html.includes(`<video id="media" src="/media/${video.id}.mp4`));
    assert.match(html, /Copy link/);
    assert.match(html, /Copy embed code/);
    assert.match(html, /Download/);
    assert.match(html, /href="https:\/\/github\.com\/aubergine-ux\/cinesnap"/);
    assert.ok(html.includes(`<meta property="og:video" content="${video.mediaUrl}">`));
    assert.ok(html.includes(`<meta name="twitter:player" content="${video.embedUrl}">`));
    assert.ok(html.includes('<meta name="twitter:card" content="player">'));

    const oembed = `${srv.base}/oembed?url=${encodeURIComponent(video.url)}&#38;format=json`;
    assert.ok(html.includes(`<link rel="alternate" type="application/json+oembed" href="${oembed}"`));
  });

  test('player page shows the view count', async () => {
    await fetch(`${srv.base}/api/videos/${video.id}/view`, { method: 'POST' });
    const html = await (await fetch(video.url)).text();
    assert.match(html, /<span id="views">1 view<\/span>/);
  });

  test('gif page uses an image', async () => {
    const html = await (await fetch(gif.url)).text();
    assert.ok(html.includes(`<img id="media" src="/media/${gif.id}.gif"`));
    assert.ok(html.includes(`<meta property="og:image" content="${gif.mediaUrl}">`));
    assert.doesNotMatch(html, /<video/);
  });

  test('normal pages cannot be framed and carry a strict CSP', async () => {
    for (const url of [video.url, `${srv.base}/`, `${srv.base}/v/zzzzzzzzzz`]) {
      const res = await fetch(url);
      await res.text();
      const csp = res.headers.get('content-security-policy');
      assert.equal(res.headers.get('x-frame-options'), 'DENY', url);
      assert.match(csp, /frame-ancestors 'none'/, url);
      assert.match(csp, /default-src 'none'/, url);
      assert.doesNotMatch(csp, /unsafe-inline/, url);
      assert.equal(res.headers.get('x-content-type-options'), 'nosniff', url);
    }
  });

  test('script and style tags carry the CSP nonce', async () => {
    const res = await fetch(video.url);
    const html = await res.text();
    const nonce = /script-src 'nonce-([^']+)'/.exec(res.headers.get('content-security-policy'))[1];
    const tags = html.match(/<(script|style)\b[^>]*>/g);
    assert.ok(tags.length >= 2);
    for (const tag of tags) assert.ok(tag.includes(`nonce="${nonce}"`), tag);
    assert.doesNotMatch(html, /\sstyle="/);

    const again = await fetch(video.url);
    await again.text();
    assert.notEqual(again.headers.get('content-security-policy'), res.headers.get('content-security-policy'));
  });

  test('embed page may be framed', async () => {
    const res = await fetch(video.embedUrl);
    assert.equal(res.status, 200);
    assert.equal(res.headers.get('x-frame-options'), null);
    assert.match(res.headers.get('content-security-policy'), /frame-ancestors \*/);
    const html = await res.text();
    assert.ok(html.includes(`<video id="media" src="/media/${video.id}.mp4`));
    assert.doesNotMatch(html, /Copy link/);
  });

  test('titles are escaped everywhere', async () => {
    for (const url of [nasty.url, nasty.embedUrl]) {
      const html = await (await fetch(url)).text();
      assert.ok(!html.includes('<script>alert'), url);
      assert.ok(!html.includes('<img src=x'), url);
      assert.ok(!html.includes(`"><img`), url);
      assert.ok(html.includes('&#60;script&#62;alert(&#34;x&#34;)&#60;/script&#62; &#38; &#39;quotes&#39;'), url);
    }

    // JSON keeps the title as it is, but the iframe snippet must be safe to paste
    const meta = await (await fetch(`${srv.base}/api/videos/${nasty.id}`)).json();
    assert.equal(meta.title, NASTY);
    assert.equal(meta.embedHtml.match(/</g).length, 2);
    assert.ok(!meta.embedHtml.includes('<script'));
  });

  test('missing video gets a friendly 404 page', async () => {
    const res = await fetch(`${srv.base}/v/zzzzzzzzzz`);
    assert.equal(res.status, 404);
    assert.match(res.headers.get('content-type'), /^text\/html/);
    const html = await res.text();
    assert.match(html, /Nothing to play here/);
    assert.match(html, /Made with Cinesnap/);

    assert.equal((await fetch(`${srv.base}/embed/zzzzzzzzzz`)).status, 404);
    assert.equal((await fetch(`${srv.base}/nope`)).status, 404);
  });

  test('oembed', async () => {
    const res = await fetch(`${srv.base}/oembed?url=${encodeURIComponent(gif.url)}&format=json`);
    assert.equal(res.status, 200);
    assert.match(res.headers.get('content-type'), /^application\/json/);
    const body = await res.json();
    assert.equal(body.version, '1.0');
    assert.equal(body.type, 'video');
    assert.equal(body.title, 'A loop');
    assert.equal(body.provider_name, 'Cinesnap');
    assert.equal(body.width, 400);
    assert.equal(body.height, 300);
    assert.ok(body.html.startsWith(`<iframe src="${gif.embedUrl}" width="400" height="300"`));
  });

  test('oembed honours maxwidth and maxheight', async () => {
    const q = `url=${encodeURIComponent(gif.url)}`;
    let body = await (await fetch(`${srv.base}/oembed?${q}&maxwidth=200`)).json();
    assert.deepEqual([body.width, body.height], [200, 150]);
    body = await (await fetch(`${srv.base}/oembed?${q}&maxwidth=1000&maxheight=60`)).json();
    assert.deepEqual([body.width, body.height], [80, 60]);
  });

  test('oembed escapes the title in its html', async () => {
    const body = await (await fetch(`${srv.base}/oembed?url=${encodeURIComponent(nasty.url)}`)).json();
    assert.equal(body.title, NASTY);
    assert.ok(!body.html.includes('<script'));
  });

  test('oembed errors', async () => {
    const q = (s) => fetch(`${srv.base}/oembed${s}`).then((r) => r.status);
    assert.equal(await q(''), 400);
    assert.equal(await q('?url=not%20a%20url'), 400);
    assert.equal(await q('?url=%E0%A4%A'), 400);
    assert.equal(await q(`?url=${encodeURIComponent(`${srv.base}/v/zzzzzzzzzz`)}`), 404);
    assert.equal(await q(`?url=${encodeURIComponent(`${srv.base}/other/${video.id}`)}`), 404);
    assert.equal(await q(`?url=${encodeURIComponent(video.url)}&format=xml`), 501);
  });
});

describe('link generation', () => {
  test('PUBLIC_URL wins over request headers', async () => {
    const srv = await startServer({ publicUrl: 'https://share.example.com' });
    try {
      const res = await upload(srv, fakeMp4(), { headers: { 'X-Forwarded-Host': 'evil.example' } });
      const body = await res.json();
      assert.equal(body.url, `https://share.example.com/v/${body.id}`);
      assert.equal(body.mediaUrl, `https://share.example.com/media/${body.id}.mp4`);
    } finally {
      await srv.stop();
    }
  });

  test('forwarded headers are used when there is no PUBLIC_URL', async () => {
    const srv = await startServer();
    try {
      let res = await upload(srv, fakeMp4(), {
        headers: { 'X-Forwarded-Proto': 'https', 'X-Forwarded-Host': 'clips.example.org' },
      });
      let body = await res.json();
      assert.equal(body.url, `https://clips.example.org/v/${body.id}`);

      // a host header that is not a host name is not echoed into links
      res = await upload(srv, fakeMp4(), { headers: { 'X-Forwarded-Host': 'a"><script>' } });
      body = await res.json();
      assert.ok(body.url.startsWith('http://localhost:'));
    } finally {
      await srv.stop();
    }
  });
});
