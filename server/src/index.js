// SC Desk sync. A dumb, encrypted relay:
//   /ch/<id>   WebSocket to a Hub (one per room or shared playlist). Every payload is an
//              opaque ciphertext made on the clients; the server never reads it. A message
//              marked `keep` is also stored (a log with sequence numbers), so a client that was
//              offline asks `?since=<seq>` and catches up. Other messages are only relayed.
//   /dir       "who is hosting a room right now": hosts announce a short-lived entry under a
//              hash of their SoundCloud user id, friends look the hashes of the people they
//              follow up. Entries expire by themselves.
import { DurableObject } from 'cloudflare:workers';

const MAX_MSG = 400 * 1024; // one message, chars (a snapshot of a long shared playlist is the big one)
const KEEP_LOG = 2000; // messages kept per channel
const BACKLOG = 500; // messages sent per connect
const IDLE_MS = 60 * 24 * 3600 * 1000; // a channel nobody touched for 60 days is deleted
const DIR_TTL_MS = 120 * 1000;
const ID_RE = /^[A-Za-z0-9_-]{16,64}$/;
const HOST_RE = /^[a-f0-9]{16,64}$/;

export class Hub extends DurableObject {
  constructor(ctx, env) {
    super(ctx, env);
    this.sql = ctx.storage.sql;
    this.sql.exec('CREATE TABLE IF NOT EXISTS log (seq INTEGER PRIMARY KEY AUTOINCREMENT, ts INTEGER NOT NULL, blob TEXT NOT NULL)');
  }

  head() {
    return this.sql.exec('SELECT COALESCE(MAX(seq), 0) AS m FROM log').one().m;
  }

  async fetch(req) {
    if (req.headers.get('Upgrade') !== 'websocket') return new Response('websocket expected', { status: 426 });
    const since = Math.max(0, parseInt(new URL(req.url).searchParams.get('since') || '0', 10) || 0);
    const { 0: client, 1: server } = new WebSocketPair();
    this.ctx.acceptWebSocket(server);
    const rows = this.sql.exec('SELECT seq, ts, blob FROM log WHERE seq > ? ORDER BY seq LIMIT ?', since, BACKLOG).toArray();
    server.send(JSON.stringify({ t: 'hello', head: this.head(), more: rows.length === BACKLOG, peers: this.ctx.getWebSockets().length }));
    for (const r of rows) server.send(JSON.stringify({ t: 'msg', seq: r.seq, ts: r.ts, blob: r.blob, keep: true }));
    await this.ctx.storage.setAlarm(Date.now() + IDLE_MS);
    return new Response(null, { status: 101, webSocket: client });
  }

  async webSocketMessage(ws, data) {
    if (typeof data !== 'string' || data.length > MAX_MSG) return;
    let m;
    try {
      m = JSON.parse(data);
    } catch {
      return;
    }
    if (m.t === 'ping') return ws.send('{"t":"pong"}');
    if (m.t !== 'send' || typeof m.blob !== 'string') return;
    const ts = Date.now();
    let seq = 0;
    if (m.keep) {
      seq = this.sql.exec('INSERT INTO log (ts, blob) VALUES (?, ?) RETURNING seq', ts, m.blob).one().seq;
      if (seq > KEEP_LOG) this.sql.exec('DELETE FROM log WHERE seq <= ?', seq - KEEP_LOG);
      await this.ctx.storage.setAlarm(Date.now() + IDLE_MS);
    }
    // everybody gets it, the sender too (that is its acknowledgement); `id` lets it recognise its own
    const out = JSON.stringify({ t: 'msg', seq, ts, blob: m.blob, keep: !!m.keep, id: typeof m.id === 'string' ? m.id.slice(0, 40) : undefined });
    for (const s of this.ctx.getWebSockets()) {
      try {
        s.send(out);
      } catch {}
    }
  }

  webSocketClose(ws, code) {
    try {
      ws.close(code);
    } catch {}
  }

  webSocketError(ws) {
    try {
      ws.close(1011);
    } catch {}
  }

  async alarm() {
    if (this.ctx.getWebSockets().length) return this.ctx.storage.setAlarm(Date.now() + IDLE_MS);
    await this.ctx.storage.deleteAll();
  }
}

export class Directory extends DurableObject {
  constructor(ctx, env) {
    super(ctx, env);
    this.sql = ctx.storage.sql;
    this.sql.exec('CREATE TABLE IF NOT EXISTS live (host TEXT PRIMARY KEY, blob TEXT NOT NULL, exp INTEGER NOT NULL)');
  }

  async fetch(req) {
    const url = new URL(req.url);
    const now = Date.now();
    this.sql.exec('DELETE FROM live WHERE exp < ?', now);
    const m = /^\/dir\/([a-f0-9]+)$/.exec(url.pathname);
    if (m && HOST_RE.test(m[1])) {
      if (req.method === 'PUT') {
        const body = await req.text();
        if (body.length > 2048) return new Response('too big', { status: 413 });
        this.sql.exec('INSERT OR REPLACE INTO live (host, blob, exp) VALUES (?, ?, ?)', m[1], body, now + DIR_TTL_MS);
        return new Response('ok');
      }
      if (req.method === 'DELETE') {
        this.sql.exec('DELETE FROM live WHERE host = ?', m[1]);
        return new Response('ok');
      }
    }
    if (url.pathname === '/dir' && req.method === 'GET') {
      const hosts = (url.searchParams.get('hosts') || '').split(',').filter((h) => HOST_RE.test(h)).slice(0, 200);
      const out = [];
      for (const h of hosts) {
        const r = this.sql.exec('SELECT blob FROM live WHERE host = ?', h).toArray()[0];
        if (r) out.push({ host: h, blob: r.blob });
      }
      return Response.json(out);
    }
    return new Response('not found', { status: 404 });
  }
}

// How many people use the program: an anonymous count. A program says once a day «a random number of this
// installation, the version, the system»: nothing about who, no address (this code does not look at it, does not
// keep it), nothing about what is played. The same number on the same day counts once.
const DAY_RE = /^\d{4}-\d{2}-\d{2}$/;
export class Stats extends DurableObject {
  constructor(ctx, env) {
    super(ctx, env);
    this.sql = ctx.storage.sql;
    this.env = env;
    this.sql.exec('CREATE TABLE IF NOT EXISTS seen (day TEXT NOT NULL, id TEXT NOT NULL, ver TEXT NOT NULL, os TEXT NOT NULL, PRIMARY KEY (day, id))');
  }

  async fetch(req) {
    const url = new URL(req.url);
    if (url.pathname === '/stat' && req.method === 'POST') {
      let b;
      try {
        b = await req.json();
      } catch {
        return new Response('bad json', { status: 400 });
      }
      const id = String(b.id || '');
      const ver = String(b.v || '').slice(0, 16);
      const os = String(b.os || '').slice(0, 12);
      if (!/^[a-f0-9]{32}$/.test(id) || !/^[\d.]+$/.test(ver) || !/^[a-z]+$/.test(os)) return new Response('bad', { status: 400 });
      const day = new Date().toISOString().slice(0, 10);
      this.sql.exec('INSERT OR REPLACE INTO seen (day, id, ver, os) VALUES (?, ?, ?, ?)', day, id, ver, os);
      // a year is plenty: older days go
      this.sql.exec("DELETE FROM seen WHERE day < date('now', '-400 day')");
      return Response.json({ ok: true });
    }
    if (url.pathname === '/stat/summary' && req.method === 'GET') {
      // only for the owner: the key is a secret of the worker (wrangler secret put STATS_KEY)
      const key = this.env.STATS_KEY;
      if (!key || url.searchParams.get('key') !== key) return new Response('no', { status: 403 });
      const one = (q, ...a) => this.sql.exec(q, ...a).toArray();
      const days = one("SELECT day, COUNT(*) AS n FROM seen WHERE day >= date('now', '-60 day') GROUP BY day ORDER BY day");
      const [w] = one("SELECT COUNT(DISTINCT id) AS n FROM seen WHERE day >= date('now', '-6 day')");
      const [m] = one("SELECT COUNT(DISTINCT id) AS n FROM seen WHERE day >= date('now', '-29 day')");
      const [all] = one('SELECT COUNT(DISTINCT id) AS n FROM seen');
      const versions = one("SELECT ver, COUNT(DISTINCT id) AS n FROM seen WHERE day >= date('now', '-6 day') GROUP BY ver ORDER BY n DESC");
      const systems = one("SELECT os, COUNT(DISTINCT id) AS n FROM seen WHERE day >= date('now', '-29 day') GROUP BY os ORDER BY n DESC");
      return Response.json({ days, week: w.n, month: m.n, all: all.n, versions, systems });
    }
    return new Response('not found', { status: 404 });
  }
}

export default {
  async fetch(req, env) {
    const url = new URL(req.url);
    const ch = /^\/ch\/([^/]+)$/.exec(url.pathname);
    if (ch) {
      if (!ID_RE.test(ch[1])) return new Response('bad channel id', { status: 400 });
      return env.HUB.get(env.HUB.idFromName(ch[1])).fetch(req);
    }
    if (url.pathname === '/stat' || url.pathname === '/stat/summary') return env.STATS.get(env.STATS.idFromName('global')).fetch(req);
    if (url.pathname.startsWith('/dir')) return env.DIR.get(env.DIR.idFromName('global')).fetch(req);
    if (url.pathname === '/' || url.pathname === '/health') return new Response('sc-desk-sync ok');
    return new Response('not found', { status: 404 });
  },
};
