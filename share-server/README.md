# Cinesnap Share

A small server you host yourself for sharing [Cinesnap](https://github.com/aubergine-ux/cinesnap) recordings. Upload an MP4 or GIF and get back a link with a clean player page, a view count and an embed code.

- One Node.js process, no npm dependencies, no build step
- Files on disk, metadata in a single SQLite file
- Player page with Open Graph / Twitter tags and oEmbed, so links unfurl in chat apps
- Uploads protected by a token; viewing needs only the link

## Quick start with Docker Compose

```sh
cd share-server
cp .env.example .env
openssl rand -hex 24          # paste the result into .env as UPLOAD_TOKEN
docker compose up -d
```

The server is now on <http://localhost:8080>. Open it to check that it is running. Until `UPLOAD_TOKEN` is set, every upload is refused.

Settings live in `.env`:

| Variable | Default | What it does |
| --- | --- | --- |
| `UPLOAD_TOKEN` | *(empty)* | Secret for uploading, listing and deleting. Uploads answer 503 while it is empty. |
| `PUBLIC_URL` | *(empty)* | Address used in generated links, e.g. `https://share.example.com`. When empty it is taken from the request (`Host`, `X-Forwarded-Proto`, `X-Forwarded-Host`). |
| `MAX_UPLOAD_MB` | `500` | Largest accepted upload. |
| `TRUST_PROXY` | *(empty)* | Number of reverse proxies in front (`1` for the usual single proxy). Makes view counting use `X-Forwarded-For`. |
| `HOST_PORT` | `8080` | Compose only: port published on the host. `127.0.0.1:8080` keeps it reachable from the host alone. |
| `PORT` | `8080` | Without Docker: port to listen on. |
| `DATA_DIR` | `./data` | Without Docker: where `videos/` and `share.db` go. |

After changing `.env`, run `docker compose up -d` again.

## Running without Docker

Needs Node.js 22.13 or newer.

```sh
cd share-server
UPLOAD_TOKEN=$(openssl rand -hex 24) PORT=8080 DATA_DIR=./data npm start
```

`npm test` runs the test suite.

## Behind a reverse proxy

Put it behind something that does HTTPS. With [Caddy](https://caddyserver.com):

```
share.example.com {
	reverse_proxy 127.0.0.1:8080
}
```

and in `.env`:

```sh
PUBLIC_URL=https://share.example.com
TRUST_PROXY=1
HOST_PORT=127.0.0.1:8080
```

If your proxy limits request bodies (nginx does, at 1 MB by default), raise that limit to at least `MAX_UPLOAD_MB`. The server expects to be at the root of its own (sub)domain, not under a path.

## Uploading

From the Cinesnap app, enter the server address and the token in the share settings. From the command line:

```sh
cinesnap share video.mp4 --server https://share.example.com --token "$UPLOAD_TOKEN"
```

Or with nothing but curl:

```sh
curl -X POST https://share.example.com/api/videos \
  -H "Authorization: Bearer $UPLOAD_TOKEN" \
  -H "Content-Type: video/mp4" \
  -H "X-Title: My%20demo" \
  --data-binary @video.mp4
```

```json
{
  "id": "qRQMReQCy8",
  "url": "https://share.example.com/v/qRQMReQCy8",
  "embedUrl": "https://share.example.com/embed/qRQMReQCy8",
  "embedHtml": "<iframe src=\"https://share.example.com/embed/qRQMReQCy8\" ...></iframe>",
  "mediaUrl": "https://share.example.com/media/qRQMReQCy8.mp4",
  "...": "plus the fields of GET /api/videos/:id"
}
```

The home page of the server has a small list of your uploads with view counts and delete buttons. It asks for the token and keeps it only for the browser session.

## API

Routes marked with a lock need `Authorization: Bearer <UPLOAD_TOKEN>`.

| Method | Path | | Description |
| --- | --- | --- | --- |
| `POST` | `/api/videos` | lock | Upload. The request body is the file itself. `Content-Type` must be `video/mp4` or `image/gif`. Optional `X-Title` header with a percent-encoded UTF-8 title. Answers `201` with the video as JSON. |
| `GET` | `/api/videos` | lock | All videos as a JSON array, newest first. |
| `GET` | `/api/videos/:id` | | One video: `id`, `title`, `type`, `bytes`, `width`, `height`, `createdAt`, `views`, `url`, `embedUrl`, `embedHtml`, `mediaUrl`. |
| `DELETE` | `/api/videos/:id` | lock | Removes the file and its record. Answers `204`. |
| `POST` | `/api/videos/:id/view` | | Counts a view and answers `{"views": n}`. The player page calls this when playback starts. |
| `GET` | `/media/:id.mp4`, `/media/:id.gif` | | The file, with range requests. |
| `GET` | `/v/:id` | | Player page. |
| `GET` | `/embed/:id` | | Bare player for iframes. |
| `GET` | `/oembed?url=…&format=json` | | oEmbed response for a player page URL. Honours `maxwidth` and `maxheight`. |
| `GET` | `/healthz` | | Answers `ok`. |

Errors come back as `{"error": "…"}` with a matching status:

| Status | When |
| --- | --- |
| `400` | Empty body, interrupted upload, bad `X-Title` encoding |
| `401` | Missing or wrong token |
| `404` | Unknown id |
| `413` | Larger than `MAX_UPLOAD_MB` |
| `415` | `Content-Type` is not MP4 or GIF, or the file contents do not match it |
| `416` | Range outside the file |
| `503` | `UPLOAD_TOKEN` is not configured |

Clients that send `Expect: 100-continue` (curl does for larger files) get the token, type and size checked before any of the file is sent.

## Views

A view is counted once per visitor per video per hour. Visitors are told apart by a salted hash of their IP address and user agent. The hash is dropped after an hour and the address itself is never stored. Behind a proxy, set `TRUST_PROXY` or every visitor looks like the proxy.

## Backups

Everything is in the data directory: `videos/` holds the files and `share.db` (with its `-wal` and `-shm` companions) holds titles, dates and view counts. With Compose it is the `cinesnap-share_share-data` volume. To copy it out:

```sh
docker compose stop
docker run --rm -v cinesnap-share_share-data:/data -v "$PWD":/backup alpine \
  tar czf /backup/cinesnap-share-backup.tgz -C /data .
docker compose start
```

To restore, unpack the archive into an empty volume the same way before starting the server. Stopping first keeps the database file consistent; if you would rather not stop it, `sqlite3 share.db ".backup copy.db"` takes a safe copy of a live database.

## Notes

- Links are unlisted, not private. Anyone who has a link can watch and download the video.
- Media files are served with a one-year cache lifetime. A deleted video disappears from the server at once but may linger in browser or CDN caches.
- There is no transcoding. What you upload is what gets played, so stick to H.264 MP4 for the widest support. Cinesnap exports are fine as they are.
