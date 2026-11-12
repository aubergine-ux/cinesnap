import { DatabaseSync } from 'node:sqlite';
import { randomBytes } from 'node:crypto';

export const VIEW_WINDOW_MS = 60 * 60 * 1000;

export function openDb(file) {
  const db = new DatabaseSync(file);
  db.exec(`
    PRAGMA journal_mode = WAL;
    PRAGMA synchronous = NORMAL;
    CREATE TABLE IF NOT EXISTS videos (
      id         TEXT PRIMARY KEY,
      title      TEXT NOT NULL,
      type       TEXT NOT NULL,
      bytes      INTEGER NOT NULL,
      width      INTEGER,
      height     INTEGER,
      created_at INTEGER NOT NULL,
      views      INTEGER NOT NULL DEFAULT 0
    );
    CREATE TABLE IF NOT EXISTS view_marks (
      video_id TEXT NOT NULL,
      visitor  TEXT NOT NULL,
      seen_at  INTEGER NOT NULL,
      PRIMARY KEY (video_id, visitor)
    ) WITHOUT ROWID;
    CREATE INDEX IF NOT EXISTS view_marks_seen ON view_marks (seen_at);
    CREATE TABLE IF NOT EXISTS meta (
      key   TEXT PRIMARY KEY,
      value TEXT NOT NULL
    );
  `);

  const q = {
    insert: db.prepare(
      'INSERT INTO videos (id, title, type, bytes, width, height, created_at) VALUES (?, ?, ?, ?, ?, ?, ?)',
    ),
    get: db.prepare('SELECT * FROM videos WHERE id = ?'),
    list: db.prepare('SELECT * FROM videos ORDER BY created_at DESC, rowid DESC'),
    remove: db.prepare('DELETE FROM videos WHERE id = ?'),
    removeMarks: db.prepare('DELETE FROM view_marks WHERE video_id = ?'),
    // Inserts a mark, or refreshes it when the old one has aged out. No change means a repeat view.
    mark: db.prepare(`
      INSERT INTO view_marks (video_id, visitor, seen_at) VALUES (?, ?, ?)
      ON CONFLICT (video_id, visitor) DO UPDATE SET seen_at = excluded.seen_at
      WHERE view_marks.seen_at <= excluded.seen_at - ${VIEW_WINDOW_MS}
    `),
    bump: db.prepare('UPDATE videos SET views = views + 1 WHERE id = ?'),
    prune: db.prepare('DELETE FROM view_marks WHERE seen_at < ?'),
    metaGet: db.prepare('SELECT value FROM meta WHERE key = ?'),
    metaSet: db.prepare('INSERT OR IGNORE INTO meta (key, value) VALUES (?, ?)'),
  };

  q.metaSet.run('visitor_salt', randomBytes(32).toString('hex'));
  const salt = q.metaGet.get('visitor_salt').value;

  return {
    salt,

    insert(v) {
      q.insert.run(v.id, v.title, v.type, v.bytes, v.width ?? null, v.height ?? null, v.createdAt);
    },

    get(id) {
      return q.get.get(id);
    },

    list() {
      return q.list.all();
    },

    remove(id) {
      q.removeMarks.run(id);
      return q.remove.run(id).changes > 0;
    },

    // Returns the view count after counting (or not counting) this visitor.
    view(id, visitor, now) {
      if (q.mark.run(id, visitor, now).changes > 0) q.bump.run(id);
      return q.get.get(id)?.views ?? 0;
    },

    prune(now) {
      q.prune.run(now - VIEW_WINDOW_MS);
    },

    close() {
      db.close();
    },
  };
}
