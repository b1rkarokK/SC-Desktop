# SC Desk sync server

Cloudflare Worker with two Durable Objects (free plan is enough):

* `/ch/<id>` - WebSocket to a channel (a room or a shared playlist). The payloads are ciphertext made
  on the clients; the server never reads them. A message sent with `keep: true` is also stored, so a
  client that was offline asks `?since=<seq>` and catches up.
* `/dir` - the short-lived list of rooms that are live right now (hosts announce, friends look up).

## Try it without an account

```
npm install
npm run dev        # http://127.0.0.1:8787, local emulation of Cloudflare
npm test           # in another terminal: 12 checks
```

## Deploy (once, free)

```
npm install
npx wrangler login     # a free Cloudflare account is needed, the browser opens
npm run deploy         # prints https://sc-desk-sync.<your-name>.workers.dev
```

Put that address into SC Desk (Settings, "Sync server").
