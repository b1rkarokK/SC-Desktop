// Integration test against `npm run dev` (local emulation of Cloudflare, no account needed).
// node test/run.mjs [http://127.0.0.1:8787]
const base = process.argv[2] || 'http://127.0.0.1:8787';
const wsBase = base.replace(/^http/, 'ws');
let failed = 0;
const ok = (cond, msg) => { console.log((cond ? 'ok   ' : 'FAIL ') + msg); if (!cond) failed++; };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

function open(id, since = 0) {
  const ws = new WebSocket(`${wsBase}/ch/${id}?since=${since}`);
  const got = [];
  ws.onmessage = (e) => got.push(JSON.parse(e.data));
  return new Promise((res, rej) => { ws.onopen = () => res({ ws, got }); ws.onerror = () => rej(new Error('ws error')); });
}

const id = 'test' + Math.random().toString(36).slice(2).padEnd(20, 'x');

// 1. relay between two live clients, the sender gets its own message back (acknowledgement)
const a = await open(id), b = await open(id);
await sleep(300);
ok(a.got[0]?.t === 'hello' && b.got[0]?.t === 'hello', 'hello on connect');
a.ws.send(JSON.stringify({ t: 'send', blob: 'ephemeral-1', id: 'm1' }));
await sleep(300);
ok(b.got.some((m) => m.t === 'msg' && m.blob === 'ephemeral-1' && m.seq === 0), 'a relayed message reaches the other client');
ok(a.got.some((m) => m.id === 'm1'), 'the sender recognises its own message');

// 2. a `keep` message is stored: a client that joins later catches up with ?since=
a.ws.send(JSON.stringify({ t: 'send', blob: 'kept-1', keep: true }));
a.ws.send(JSON.stringify({ t: 'send', blob: 'kept-2', keep: true }));
await sleep(300);
b.ws.close();
a.ws.send(JSON.stringify({ t: 'send', blob: 'kept-3-while-b-is-away', keep: true }));
await sleep(300);
const late = await open(id, 0);
await sleep(400);
const blobs = late.got.filter((m) => m.t === 'msg').map((m) => m.blob);
ok(JSON.stringify(blobs) === JSON.stringify(['kept-1', 'kept-2', 'kept-3-while-b-is-away']), 'a late client gets the stored messages in order, not the ephemeral one');
const seqs = late.got.filter((m) => m.t === 'msg').map((m) => m.seq);
ok(seqs[0] < seqs[1] && seqs[1] < seqs[2], 'sequence numbers grow');
const back = await open(id, seqs[1]);
await sleep(300);
ok(back.got.filter((m) => m.t === 'msg').map((m) => m.blob).join() === 'kept-3-while-b-is-away', '?since= returns only what is newer');
ok(late.got[0].head === seqs[2], 'hello carries the head sequence number');

// 3. limits
a.ws.send(JSON.stringify({ t: 'send', blob: 'x'.repeat(30000), keep: true }));
await sleep(300);
ok(!a.got.some((m) => m.blob && m.blob.length > 25000), 'an oversized message is dropped');
const bad = await fetch(`${base}/ch/short`);
ok(bad.status === 400, 'a bad channel id is refused');

// 4. directory of live rooms
const host = 'ab'.repeat(16);
let r = await fetch(`${base}/dir/${host}`, { method: 'PUT', body: 'room-blob' });
ok(r.ok, 'a host announces a room');
r = await fetch(`${base}/dir?hosts=${host},${'cd'.repeat(16)}`);
const list = await r.json();
ok(list.length === 1 && list[0].host === host && list[0].blob === 'room-blob', 'friends see only the hosts that are live');
await fetch(`${base}/dir/${host}`, { method: 'DELETE' });
r = await fetch(`${base}/dir?hosts=${host}`);
ok((await r.json()).length === 0, 'closing the room removes the entry');

for (const c of [a, late, back]) c.ws.close();
console.log(failed ? `${failed} FAILED` : 'all passed');
process.exit(failed ? 1 : 0);
