// Live updates. Each signed-in app keeps one WebSocket open to its account's Durable Object.
// When something an account sees changes (its computers, its friends, their computers, a
// request), the service sends "changed" and the app fetches its lists again at once. The
// message carries no data: the app reads everything through the usual signed-in calls.

const MAX_SOCKETS = 24;

export class Events {
  constructor(private state: DurableObjectState) {
    // Keep-alives are answered without waking the object.
    this.state.setWebSocketAutoResponse(new WebSocketRequestResponsePair("ping", "pong"));
  }
  async fetch(request: Request): Promise<Response> {
    if (request.headers.get("upgrade")?.toLowerCase() === "websocket") {
      const open = this.state.getWebSockets();
      // A runaway client cannot pile up sockets: the oldest go first.
      for (const old of open.slice(0, Math.max(0, open.length - MAX_SOCKETS + 1))) {
        try { old.close(1008, "too many"); } catch { /* already gone */ }
      }
      const pair = new WebSocketPair();
      this.state.acceptWebSocket(pair[1]);
      return new Response(null, { status: 101, webSocket: pair[0] });
    }
    for (const socket of this.state.getWebSockets()) {
      try { socket.send("changed"); } catch { /* closing */ }
    }
    return new Response(null, { status: 204 });
  }
  webSocketMessage(): void {}
  webSocketClose(socket: WebSocket, code: number): void {
    try { socket.close(code === 1005 ? 1000 : code, ""); } catch { /* already closed */ }
  }
  webSocketError(): void {}
}

export interface EventsEnv {
  DB: D1Database;
  EVENTS?: DurableObjectNamespace;
}

/** Everyone in a friendship with the account, accepted or not. */
export async function related(env: EventsEnv, account: string): Promise<string[]> {
  const rows = await env.DB.prepare("SELECT CASE WHEN a=? THEN b ELSE a END AS other FROM friends WHERE a=? OR b=?")
    .bind(account, account, account).all<{ other: string }>();
  return rows.results.map((r) => r.other);
}

/** Tells each account's open apps to look again. Best effort: polling still catches up. */
export async function poke(env: EventsEnv, accounts: Iterable<string>): Promise<void> {
  const ns = env.EVENTS;
  if (!ns) return;
  await Promise.allSettled([...new Set(accounts)].map((id) =>
    ns.get(ns.idFromName(id)).fetch("https://events/changed", { method: "POST" })));
}

/** Opens an app's live channel, for an account already signed in. */
export function connect(env: EventsEnv, account: string, request: Request): Promise<Response> | Response {
  const ns = env.EVENTS;
  if (!ns) return new Response(JSON.stringify({ error: "not_found" }), { status: 404, headers: { "content-type": "application/json" } });
  return ns.get(ns.idFromName(account)).fetch(request);
}
