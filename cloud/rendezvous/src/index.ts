// PC Remote rendezvous: one Durable Object per PC relays agent frames between the PC's host and
// its phones, and hands out TURN credentials. Protocol: docs/remote-protocol.md.
import { DurableObject } from "cloudflare:workers";
import { b64urlDecode, freshTs, hostSig, phoneSig, sameHex } from "./auth";
import { firstFrameOk, isWelcome, MAX_GUESTS, MAX_PENDING_GUESTS, PENDING_SECONDS } from "./guest";

export interface Env {
  ROOM: DurableObjectNamespace<Room>;
  TURN_KEY_ID?: string;
  TURN_KEY_TOKEN?: string;
}

const ID = /^[A-Za-z0-9_-]{1,64}$/;

export default {
  async fetch(req: Request, env: Env): Promise<Response> {
    const url = new URL(req.url);
    if (!["/v1/host", "/v1/phone", "/v1/guest", "/v1/turn"].includes(url.pathname)) {
      return new Response("PC Remote rendezvous", { status: url.pathname === "/" ? 200 : 404 });
    }
    const pc = url.searchParams.get("pc") ?? "";
    if (!ID.test(pc)) return new Response("bad pc", { status: 400 });
    return env.ROOM.get(env.ROOM.idFromName(pc)).fetch(req);
  },
} satisfies ExportedHandler<Env>;

type Attach = { role: "host" } | { role: "phone"; cid: number; dev: string; guest?: { since: number; authed: boolean } };

export class Room extends DurableObject<Env> {
  constructor(ctx: DurableObjectState, env: Env) {
    super(ctx, env);
    ctx.setWebSocketAutoResponse(new WebSocketRequestResponsePair("ping", "pong"));
  }

  private async key(): Promise<Uint8Array | null> {
    const k = await this.ctx.storage.get<string>("relayKey");
    return k ? b64urlDecode(k) : null;
  }

  private host(): WebSocket | null {
    const hs = this.ctx.getWebSockets("host");
    return hs[hs.length - 1] ?? null; // newest wins while an old one is closing
  }

  private phones(): WebSocket[] {
    return this.ctx.getWebSockets("phone");
  }

  async fetch(req: Request): Promise<Response> {
    const url = new URL(req.url);
    const q = (k: string) => url.searchParams.get(k);
    const pc = q("pc")!;
    const ts = q("ts");
    const now = Math.floor(Date.now() / 1000);
    if (url.pathname === "/v1/guest") return this.guest(req, now);
    if (!freshTs(ts, now)) return new Response("stale", { status: 401 });

    if (url.pathname === "/v1/host") {
      let key = await this.key();
      let register: Uint8Array | null = null;
      if (!key) {
        register = b64urlDecode(q("register") ?? "");
        if (!register || register.length !== 32) return new Response("unknown pc", { status: 401 });
        key = register;
      }
      if (!sameHex(q("sig") ?? "", await hostSig(key, pc, ts!))) return new Response("bad sig", { status: 401 });
      if (req.headers.get("Upgrade") !== "websocket") return new Response("websocket only", { status: 426 });
      if (register) await this.ctx.storage.put("relayKey", q("register")!);
      // a new host socket replaces the old one, with its phones
      for (const ws of [...this.ctx.getWebSockets("host"), ...this.phones()]) ws.close(4409, "replaced");
      const [client, server] = Object.values(new WebSocketPair());
      this.ctx.acceptWebSocket(server, ["host"]);
      server.serializeAttachment({ role: "host" } satisfies Attach);
      return new Response(null, { status: 101, webSocket: client });
    }

    // the host's own TURN credentials (both ends relay when NATs block a direct path)
    if (url.pathname === "/v1/turn" && q("role") === "host") {
      const key = await this.key();
      if (!key || !sameHex(q("sig") ?? "", await hostSig(key, pc, ts!))) return new Response("bad sig", { status: 401 });
      return this.turn();
    }

    // phone endpoints
    const dev = q("dev") ?? "";
    const key = await this.key();
    if (!key || !ID.test(dev)) return new Response("unknown", { status: 401 });
    if (!sameHex(q("sig") ?? "", await phoneSig(key, pc, dev, ts!))) return new Response("bad sig", { status: 401 });
    const revoked = (await this.ctx.storage.get<string[]>("revoked")) ?? [];
    if (revoked.includes(dev)) return new Response("revoked", { status: 403 });

    if (url.pathname === "/v1/turn") return this.turn();

    if (req.headers.get("Upgrade") !== "websocket") return new Response("websocket only", { status: 426 });
    const [client, server] = Object.values(new WebSocketPair());
    const host = this.host();
    if (!host) {
      server.accept();
      server.close(4404, "pc_offline");
      return new Response(null, { status: 101, webSocket: client });
    }
    const cid = ((await this.ctx.storage.get<number>("nextCid")) ?? 0) + 1;
    await this.ctx.storage.put("nextCid", cid);
    this.ctx.acceptWebSocket(server, ["phone", `c${cid}`]);
    server.serializeAttachment({ role: "phone", cid, dev } satisfies Attach);
    host.send(JSON.stringify({ c: cid, o: "open" }));
    return new Response(null, { status: 101, webSocket: client });
  }

  private async turn(): Promise<Response> {
    const { TURN_KEY_ID: id, TURN_KEY_TOKEN: token } = this.env;
    if (!id || !token) return new Response("relay not configured", { status: 503 });
    const r = await fetch(`https://rtc.live.cloudflare.com/v1/turn/keys/${id}/credentials/generate-ice-servers`, {
      method: "POST",
      headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/json" },
      body: JSON.stringify({ ttl: 6 * 3600 }),
    });
    if (!r.ok) return new Response("relay error", { status: 502 });
    return new Response(await r.text(), { headers: { "Content-Type": "application/json" } });
  }

  // A friend's socket. No ticket: the host checks the signed grant in the first frame.
  private async guest(req: Request, now: number): Promise<Response> {
    if (req.headers.get("Upgrade") !== "websocket") return new Response("websocket only", { status: 426 });
    if (!(await this.key())) return new Response("unknown pc", { status: 404 });
    const open = this.ctx.getWebSockets("guest");
    const pending = open.filter((g) => !((g.deserializeAttachment() as Attach & { guest?: { authed: boolean } }).guest?.authed));
    if (open.length >= MAX_GUESTS || pending.length >= MAX_PENDING_GUESTS) return new Response("busy", { status: 429 });
    const [client, server] = Object.values(new WebSocketPair());
    const host = this.host();
    if (!host) {
      server.accept();
      server.close(4404, "pc_offline");
      return new Response(null, { status: 101, webSocket: client });
    }
    const cid = ((await this.ctx.storage.get<number>("nextCid")) ?? 0) + 1;
    await this.ctx.storage.put("nextCid", cid);
    this.ctx.acceptWebSocket(server, ["phone", "guest", `c${cid}`]);
    server.serializeAttachment({ role: "phone", cid, dev: "", guest: { since: now, authed: false } } satisfies Attach);
    host.send(JSON.stringify({ c: cid, o: "open" }));
    await this.ctx.storage.setAlarm(Date.now() + PENDING_SECONDS * 1000);
    return new Response(null, { status: 101, webSocket: client });
  }

  // Closes guest sockets the host never accepted.
  async alarm(): Promise<void> {
    const now = Math.floor(Date.now() / 1000);
    let waiting = false;
    for (const g of this.ctx.getWebSockets("guest")) {
      const a = g.deserializeAttachment() as Attach & { guest?: { since: number; authed: boolean } };
      if (a.guest?.authed) continue;
      if (now - (a.guest?.since ?? 0) >= PENDING_SECONDS - 1) g.close(4408, "not accepted");
      else waiting = true;
    }
    if (waiting) await this.ctx.storage.setAlarm(Date.now() + 2000);
  }

  async webSocketMessage(ws: WebSocket, msg: string | ArrayBuffer): Promise<void> {
    const a = ws.deserializeAttachment() as Attach;
    if (a.role === "phone") {
      if (a.guest && !a.guest.authed && !firstFrameOk(msg)) return ws.close(4401, "unauthorized");
      const host = this.host();
      if (!host) return ws.close(4404, "pc_offline");
      if (typeof msg === "string") host.send(JSON.stringify({ c: a.cid, m: msg }));
      else {
        const out = new Uint8Array(4 + msg.byteLength);
        new DataView(out.buffer).setUint32(0, a.cid);
        out.set(new Uint8Array(msg), 4);
        host.send(out);
      }
      return;
    }
    // from the host
    if (typeof msg !== "string") {
      if (msg.byteLength < 4) return;
      const cid = new DataView(msg).getUint32(0);
      this.ctx.getWebSockets(`c${cid}`)[0]?.send(msg.slice(4));
      return;
    }
    let e: { c?: number; o?: string; m?: string; dev?: string };
    try { e = JSON.parse(msg); } catch { return; }
    if (e.o === "revoke" && typeof e.dev === "string" && ID.test(e.dev)) {
      const revoked = new Set((await this.ctx.storage.get<string[]>("revoked")) ?? []);
      revoked.add(e.dev);
      await this.ctx.storage.put("revoked", [...revoked]);
      for (const p of this.phones()) if ((p.deserializeAttachment() as Attach & { dev?: string }).dev === e.dev) p.close(4403, "revoked");
      return;
    }
    const p = typeof e.c === "number" ? this.ctx.getWebSockets(`c${e.c}`)[0] : undefined;
    if (!p) return;
    if (e.o === "close") p.close(1000, "closed by pc");
    else if (typeof e.m === "string") {
      const pa = p.deserializeAttachment() as Attach;
      if (pa.role === "phone" && pa.guest && !pa.guest.authed && isWelcome(e.m)) {
        p.serializeAttachment({ ...pa, guest: { ...pa.guest, authed: true } } satisfies Attach);
      }
      p.send(e.m);
    }
  }

  async webSocketClose(ws: WebSocket, code: number): Promise<void> {
    const a = ws.deserializeAttachment() as Attach;
    if (a.role === "phone") this.host()?.send(JSON.stringify({ c: a.cid, o: "close" }));
    else if (!this.ctx.getWebSockets("host").some((h) => h !== ws)) for (const p of this.phones()) p.close(4404, "pc_offline");
    try { ws.close(code === 1005 ? 1000 : code, "bye"); } catch { /* already closed */ }
  }

  async webSocketError(ws: WebSocket): Promise<void> {
    await this.webSocketClose(ws, 1011);
  }
}
