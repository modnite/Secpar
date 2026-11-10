import { hostRoutes } from "./hosts";
import { EFFECTIVE, flags, friendSettings, saveSettings } from "./settings";
import { passwordRoutes } from "./passwords";
import { SignJWT, importJWK, calculateJwkThumbprint, type JWK } from "jose";

export interface Env {
  DB: D1Database;
  PUBLIC_ORIGIN: string;
  GRANT_PRIVATE_JWK: string;
  RESEND_API_KEY?: string;
  EMAIL_FROM?: string;
}
type Account = { id: string; handle: string; name: string; visible: number };
type Grant = {
  id: string;
  computer: string;
  owner: string;
  guest: string;
  requested: string;
  approved: string | null;
  state: string;
  expires: number;
};
const permissions = new Set([
  "view",
  "audio",
  "gamepad",
  "keyboard",
  "mouse",
  "clipboard",
  "files",
]);
const STATE = "__Host-secpar-oauth";
const seconds = () => Math.floor(Date.now() / 1000);
const random = () =>
  Array.from(crypto.getRandomValues(new Uint8Array(32)), (b) =>
    b.toString(16).padStart(2, "0"),
  ).join("");
const response = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), {
    status,
    headers: {
      "content-type": "application/json",
      "cache-control": "no-store",
      "x-content-type-options": "nosniff",
    },
  });
function deny(status: number, error: string): never {
  throw new ApiError(status, error);
}
class ApiError extends Error {
  constructor(
    public status: number,
    message: string,
  ) {
    super(message);
  }
}
const q = (env: Env, sql: string, ...values: (string | number | null)[]) =>
  env.DB.prepare(sql).bind(...values);
const pair = (a: string, b: string) => (a < b ? [a, b] : [b, a]);
function origin(env: Env): string {
  const url = new URL(env.PUBLIC_ORIGIN);
  if (
    url.protocol !== "https:" ||
    url.hostname.endsWith(".invalid") ||
    url.username ||
    url.password ||
    url.pathname !== "/" ||
    url.search ||
    url.hash
  )
    deny(503, "account_service_not_configured");
  return url.origin;
}
export async function hash(value: string): Promise<string> {
  return Array.from(
    new Uint8Array(
      await crypto.subtle.digest("SHA-256", new TextEncoder().encode(value)),
    ),
  )
    .map((b) => b.toString(16).padStart(2, "0"))
    .join("");
}
async function body(request: Request): Promise<Record<string, unknown>> {
  if (Number(request.headers.get("content-length") ?? 0) > 8192)
    deny(413, "request_too_large");
  // Bound streaming bodies too: Content-Length is not trusted.
  const reader = request.body?.getReader();
  if (!reader) deny(400, "body_required");
  let total = 0;
  const chunks: Uint8Array[] = [];
  while (true) {
    const next = await reader.read();
    if (next.done) break;
    total += next.value.length;
    if (total > 8192) {
      await reader.cancel();
      deny(413, "request_too_large");
    }
    chunks.push(next.value);
  }
  const data = new Uint8Array(total);
  let at = 0;
  for (const c of chunks) {
    data.set(c, at);
    at += c.length;
  }
  const decoded = new TextDecoder().decode(data);
  try {
    const v: unknown = JSON.parse(decoded);
    if (!v || typeof v !== "object" || Array.isArray(v))
      deny(400, "invalid_json");
    return v as Record<string, unknown>;
  } catch {
    return deny(400, "invalid_json");
  }
}
function text(value: unknown, max = 80): string {
  if (typeof value !== "string" || !value.trim() || value.length > max)
    deny(400, "invalid_value");
  return value.trim();
}
function perms(value: unknown): string[] {
  if (
    !Array.isArray(value) ||
    !value.length ||
    value.length > 7 ||
    !value.every((p) => typeof p === "string" && permissions.has(p))
  )
    deny(400, "invalid_permissions");
  return [...new Set(value as string[])].sort();
}
async function user(request: Request, env: Env): Promise<Account> {
  const token = request.headers
    .get("authorization")
    ?.match(/^Bearer ([a-f0-9]{64})$/)?.[1];
  if (!token) deny(401, "sign_in_required");
  const row = await q(
    env,
    "SELECT a.id,a.handle,a.name,a.visible FROM sessions s JOIN accounts a ON a.id=s.user_id WHERE s.hash=? AND s.expires>?",
    await hash(token),
    seconds(),
  ).first<Account>();
  return row ?? deny(401, "session_expired");
}
async function blocked(env: Env, a: string, b: string): Promise<boolean> {
  return !!(await q(
    env,
    "SELECT 1 FROM blocks WHERE (owner=? AND blocked=?) OR (owner=? AND blocked=?)",
    a,
    b,
    b,
    a,
  ).first());
}
async function friends(env: Env, a: string, b: string): Promise<boolean> {
  const [low, high] = pair(a, b);
  return (
    !(await blocked(env, a, b)) &&
    !!(await q(
      env,
      "SELECT 1 FROM friends WHERE a=? AND b=? AND state='accepted'",
      low,
      high,
    ).first())
  );
}
async function signing(env: Env) {
  let jwk: JWK;
  try {
    jwk = JSON.parse(env.GRANT_PRIVATE_JWK);
  } catch {
    return deny(503, "signing_not_configured");
  }
  if (jwk.kty !== "EC" || jwk.crv !== "P-256" || !jwk.d || !jwk.x || !jwk.y)
    deny(503, "invalid_signing_key");
  const publicKey: JWK = {
    kty: "EC",
    crv: "P-256",
    x: jwk.x,
    y: jwk.y,
    alg: "ES256",
    use: "sig",
  };
  const kid = await calculateJwkThumbprint(publicKey);
  return {
    privateKey: await importJWK(jwk, "ES256"),
    publicKey: { ...publicKey, kid },
    kid,
  };
}
// Usernames are 3 to 30 letters, digits or underscores and are unique without regard to case.
const USERNAME = /^[a-z0-9_]{3,30}$/;
async function limit(
  request: Request,
  env: Env,
  operation: string,
  max: number,
) {
  const key = `${operation}:${Math.floor(seconds() / 60)}:${await hash(request.headers.get("cf-connecting-ip") ?? "local")}`;
  const row = await q(
    env,
    "INSERT INTO rate_limits(key,count,expires) VALUES(?,1,?) ON CONFLICT(key) DO UPDATE SET count=count+1 RETURNING count",
    key,
    seconds() + 120,
  ).first<{ count: number }>();
  if (!row || row.count > max) deny(429, "rate_limited");
}

export async function handle(
  request: Request,
  env: Env,
  providerFetch: typeof fetch = fetch,
): Promise<Response> {
  try {
    const url = new URL(request.url);
    const path = url.pathname;
    const method = request.method;
    if (path === "/health" && method === "GET")
      return response({ service: "secpar-accounts", version: 1 });
    const base = origin(env);
    if (url.origin !== base) deny(400, "wrong_origin");
    if (path === "/.well-known/jwks.json" && method === "GET") {
      const key = await signing(env);
      return response({ keys: [key.publicKey] });
    }
    const passwordResponse = await passwordRoutes(request, env, providerFetch);
    if (passwordResponse) return passwordResponse;
    const hostApi = {body,fail:deny,hash,limit:(r:Request,_env:unknown,operation:string,maximum:number)=>limit(r,env,operation,maximum)};
    const machineResponse=await hostRoutes(request,env,null,hostApi);
    if(machineResponse)return machineResponse;
    const me = await user(request, env);
    const ownerHostResponse=await hostRoutes(request,env,me,hostApi);
    if(ownerHostResponse)return ownerHostResponse;
    if (path === "/v1/me" && method === "GET")
      return response({ ...me, visible: !!me.visible });
    if (path === "/v1/me/visibility" && method === "PUT") {
      const visible = (await body(request)).visible;
      if (typeof visible !== "boolean") deny(400, "invalid_value");
      await q(env, "UPDATE accounts SET visible=? WHERE id=?", visible ? 1 : 0, me.id).run();
      return response({ visible });
    }
    if (path === "/v1/directory" && method === "GET") {
      await limit(request, env, "directory", 60);
      const search = (url.searchParams.get("q") ?? "").trim().replace(/^@/, "").toLowerCase();
      const after = (url.searchParams.get("after") ?? "").toLowerCase();
      if (
        (search && !/^[a-z0-9_]{1,30}$/.test(search)) ||
        (after && !/^[a-z0-9_]{1,30}$/.test(after))
      )
        return response([]);
      const rows = await q(
        env,
        "SELECT x.id,x.handle,x.name,x.handle_lower AS lower,f.state,f.requester FROM accounts x LEFT JOIN friends f ON f.a=MIN(x.id,?) AND f.b=MAX(x.id,?) WHERE x.visible=1 AND x.id<>? AND x.handle_lower LIKE ? ESCAPE '!' AND x.handle_lower>? AND NOT EXISTS(SELECT 1 FROM blocks z WHERE (z.owner=? AND z.blocked=x.id) OR (z.owner=x.id AND z.blocked=?)) ORDER BY x.handle_lower LIMIT 20",
        me.id,
        me.id,
        me.id,
        search.replaceAll("_", "!_") + "%",
        after,
        me.id,
        me.id,
      ).all<{ id: string; handle: string; name: string; lower: string; state: string | null; requester: string | null }>();
      return response(
        rows.results.map((r) => ({
          handle: r.handle,
          name: r.name,
          cursor: r.lower,
          relation: !r.state ? "none" : r.state === "accepted" ? "friend" : r.requester === me.id ? "requested" : "incoming",
        })),
      );
    }
    if (path === "/auth/logout" && method === "POST") {
      const token = request.headers.get("authorization")?.match(/^Bearer ([a-f0-9]{64})$/)?.[1] ?? "";
      await q(env, "DELETE FROM sessions WHERE hash=?", await hash(token)).run();
      return new Response(null, { status: 204, headers: { "cache-control": "no-store" } });
    }
    if (path === "/v1/friends" && method === "GET") {
      const rows = await q(
        env,
        "SELECT x.id,x.handle,x.name,f.state,f.requester,COALESCE(s.input,'view') AS input,COALESCE(s.connect,'ask') AS connect,"+EFFECTIVE+" FROM friends f JOIN accounts x ON x.id=CASE WHEN f.a=? THEN f.b ELSE f.a END LEFT JOIN friend_settings s ON s.owner=? AND s.friend=x.id WHERE (f.a=? OR f.b=?) AND NOT EXISTS(SELECT 1 FROM blocks z WHERE (z.owner=? AND z.blocked=x.id) OR (z.owner=x.id AND z.blocked=?))",
        me.id,
        me.id,
        me.id,
        me.id,
        me.id,
        me.id,
      ).all();
      return response(flags(rows.results));
    }
    const settings = path.match(/^\/v1\/friends\/([^/]+)\/settings$/);
    if (settings && ["GET", "PUT"].includes(method)) {
      const other = settings[1];
      if (!(await friends(env, me.id, other))) deny(404, "account_unavailable");
      if (method === "PUT") {
        if (!(await saveSettings(env, me.id, other, await body(request)))) deny(400, "invalid_value");
      }
      return response(await friendSettings(env, me.id, other));
    }
    const byName = path.match(/^\/v1\/friends\/by-username\/([^/]+)$/);
    if (byName && method === "PUT") {
      await limit(request, env, "friend", 20);
      let name = "";
      try {
        name = decodeURIComponent(byName[1]).trim().replace(/^@/, "").toLowerCase();
      } catch {
        // An undecodable name matches nobody.
      }
      // Unknown, blocked, self and malformed names all answer the same way.
      const other = USERNAME.test(name)
        ? await q(
            env,
            "SELECT id FROM accounts WHERE handle_lower=?",
            name,
          ).first<{ id: string }>()
        : null;
      if (!other || other.id === me.id || (await blocked(env, me.id, other.id)))
        deny(404, "account_unavailable");
      const [a, b] = pair(me.id, other.id);
      await q(
        env,
        "INSERT INTO friends(a,b,requester,state) VALUES(?,?,?,'pending') ON CONFLICT(a,b) DO NOTHING",
        a,
        b,
        me.id,
      ).run();
      return response({ state: "pending" }, 202);
    }
    const friend = path.match(/^\/v1\/friends\/([^/]+)(?:\/(accept))?$/);
    if (friend) {
      const other = friend[1];
      if (other === me.id) deny(400, "cannot_friend_yourself");
      if (
        !(await q(env, "SELECT 1 FROM accounts WHERE id=?", other).first()) ||
        (await blocked(env, me.id, other))
      )
        deny(404, "account_unavailable");
      const [a, b] = pair(me.id, other);
      if (method === "PUT" && !friend[2]) {
        await limit(request, env, "friend", 20);
        await q(
          env,
          "INSERT INTO friends(a,b,requester,state) VALUES(?,?,?,'pending') ON CONFLICT(a,b) DO NOTHING",
          a,
          b,
          me.id,
        ).run();
        return response({ state: "pending" }, 202);
      }
      if (method === "POST" && friend[2]) {
        const changed = await q(
          env,
          "UPDATE friends SET state='accepted' WHERE a=? AND b=? AND requester<>? AND state='pending'",
          a,
          b,
          me.id,
        ).run();
        if (!changed.meta.changes) deny(404, "incoming_request_not_found");
        return response({ state: "accepted" });
      }
      if (method === "DELETE" && !friend[2]) {
        await env.DB.batch([
          q(env, "DELETE FROM friends WHERE a=? AND b=?", a, b),
          q(
            env,
            "UPDATE connection_requests SET state='revoked' WHERE (owner=? AND guest=?) OR (owner=? AND guest=?)",
            me.id,
            other,
            other,
            me.id,
          ),
        ]);
        return response({ removed: true });
      }
    }
    if (path === "/v1/blocks" && method === "GET") {
      return response(
        (
          await q(
            env,
            "SELECT a.id,a.handle,a.name FROM blocks b JOIN accounts a ON a.id=b.blocked WHERE b.owner=? ORDER BY a.handle",
            me.id,
          ).all()
        ).results,
      );
    }
    const block = path.match(/^\/v1\/blocks\/([^/]+)$/);
    if (block && method === "DELETE") {
      await q(
        env,
        "DELETE FROM blocks WHERE owner=? AND blocked=?",
        me.id,
        block[1],
      ).run();
      return response({ unblocked: true });
    }
    if (block && method === "PUT") {
      const other = block[1];
      if (other === me.id) deny(400, "cannot_block_yourself");
      if (!(await q(env, "SELECT 1 FROM accounts WHERE id=?", other).first()))
        deny(404, "account_unavailable");
      const [a, b] = pair(me.id, other);
      await env.DB.batch([
        q(
          env,
          "INSERT INTO blocks(owner,blocked) VALUES(?,?) ON CONFLICT DO NOTHING",
          me.id,
          other,
        ),
        q(env, "DELETE FROM friends WHERE a=? AND b=?", a, b),
        q(
          env,
          "UPDATE connection_requests SET state='revoked' WHERE (owner=? AND guest=?) OR (owner=? AND guest=?)",
          me.id,
          other,
          other,
          me.id,
        ),
      ]);
      return response({ blocked: true });
    }
    if (path === "/v1/computers") {
      if (method === "GET")
        return response(
          (
            await q(
              env,
              "SELECT id,name,platform FROM computers WHERE owner=?",
              me.id,
            ).all()
          ).results,
        );
      if (method === "POST") {
        const b = await body(request),
          name = text(b.name),
          platform = text(b.platform);
        if (!["windows", "linux", "macos"].includes(platform))
          deny(400, "invalid_host_platform");
        const id = crypto.randomUUID();
        await q(
          env,
          "INSERT INTO computers(id,owner,name,platform,created) VALUES(?,?,?,?,?)",
          id,
          me.id,
          name,
          platform,
          seconds(),
        ).run();
        return response({ id, name, platform }, 201);
      }
    }
    if (path === "/v1/requests" && method === "POST") {
      await limit(request, env, "connection", 20);
      const b = await body(request);
      const computer = text(b.computer),
        requested = perms(b.permissions);
      const host = await q(
        env,
        "SELECT owner FROM computers WHERE id=?",
        computer,
      ).first<{ owner: string }>();
      if (!host || !(await friends(env, host.owner, me.id)))
        deny(403, "friend_access_required");
      const policy = await friendSettings(env, host.owner, me.id);
      // A view only friend never gets keyboard or mouse, whatever the app asks for.
      const allowed = requested.filter((p) => (p !== "keyboard" || policy.keyboard) && (p !== "mouse" || policy.mouse));
      if (!allowed.length) deny(403, "input_not_allowed");
      const id = crypto.randomUUID();
      const auto = policy.connect === "always";
      await q(
        env,
        "INSERT INTO connection_requests(id,computer,owner,guest,requested,approved,state,expires) VALUES(?,?,?,?,?,?,?,?)",
        id,
        computer,
        host.owner,
        me.id,
        JSON.stringify(allowed),
        auto ? JSON.stringify(allowed) : null,
        auto ? "approved" : "pending",
        seconds() + 300,
      ).run();
      return response(
        { id, state: auto ? "approved" : "pending", expires_in: 300, permissions: allowed },
        201,
      );
    }
    if (path === "/v1/requests" && method === "GET")
      return response(
        (
          await q(
            env,
            "SELECT r.*,g.handle AS guest_handle,c.name AS computer_name FROM connection_requests r JOIN accounts g ON g.id=r.guest LEFT JOIN computers c ON c.id=r.computer WHERE (r.owner=? OR r.guest=?) AND r.expires>? ORDER BY r.expires",
            me.id,
            me.id,
            seconds(),
          ).all()
        ).results,
      );
    const grant = path.match(
      /^\/v1\/requests\/([^/]+)(?:\/(approve|claim|revoke))?$/,
    );
    if (grant) {
      const row = await q(
        env,
        "SELECT * FROM connection_requests WHERE id=? AND (owner=? OR guest=?)",
        grant[1],
        me.id,
        me.id,
      ).first<Grant>();
      if (!row) deny(404, "request_not_found");
      const active =
        row.expires > seconds() && (await friends(env, row.owner, row.guest));
      if (method === "GET" && !grant[2])
        return response({ ...row, active: active && row.state === "claimed" });
      if (method === "POST" && grant[2] === "approve") {
        if (row.owner !== me.id) deny(403, "host_owner_required");
        if (!active || row.state !== "pending")
          deny(409, "request_not_pending");
        const approved = perms((await body(request)).permissions),
          requested = new Set(JSON.parse(row.requested) as string[]);
        if (!approved.every((p) => requested.has(p)))
          deny(400, "cannot_expand_requested_permissions");
        const may = await friendSettings(env, row.owner, row.guest);
        if (approved.some((p) => (p === "keyboard" && !may.keyboard) || (p === "mouse" && !may.mouse)))
          deny(400, "input_not_allowed");
        const changed = await q(
          env,
          "UPDATE connection_requests SET state='approved',approved=? WHERE id=? AND state='pending' AND expires>?",
          JSON.stringify(approved),
          row.id,
          seconds(),
        ).run();
        if (!changed.meta.changes) deny(409, "request_not_pending");
        return response({ state: "approved" });
      }
      if (method === "POST" && grant[2] === "claim") {
        if (row.guest !== me.id) deny(403, "intended_guest_required");
        if (!active) deny(403, "access_expired_or_revoked");
        // The host must be online, switched on for friends, and have said where to reach it.
        const host = await q(
          env,
          "SELECT pc,relay,certificate_fp FROM host_keys WHERE computer=? AND sharing=1 AND last_seen>? AND pc IS NOT NULL AND relay IS NOT NULL",
          row.computer,
          seconds() - 90,
        ).first<{ pc: string; relay: string; certificate_fp: string }>();
        if (!host) deny(409, "host_not_sharing");
        const key = await signing(env);
        const claimed = await q(
          env,
          "UPDATE connection_requests SET state='claimed',expires=? WHERE id=? AND state='approved' AND expires>? RETURNING approved",
          seconds() + 8 * 3600,
          row.id,
          seconds(),
        ).first<{ approved: string }>();
        if (!claimed) deny(409, "grant_not_available");
        const expires = Math.min(seconds() + 60, row.expires);
        const token = await new SignJWT({
          permissions: JSON.parse(claimed.approved),
        })
          .setProtectedHeader({ alg: "ES256", kid: key.kid, typ: "JWT" })
          .setIssuer(base)
          .setAudience(row.computer)
          .setSubject(row.guest)
          .setJti(row.id)
          .setIssuedAt()
          .setExpirationTime(expires)
          .sign(key.privateKey);
        return response({
          grant: token,
          expires_at: expires,
          host: { pc: host.pc, relay: host.relay, fingerprint: host.certificate_fp },
        });
      }
      if (method === "POST" && grant[2] === "revoke") {
        await q(
          env,
          "UPDATE connection_requests SET state='revoked' WHERE id=?",
          row.id,
        ).run();
        return response({ state: "revoked" });
      }
    }
    return response({ error: "not_found" }, 404);
  } catch (error) {
    if (error instanceof ApiError)
      return response({ error: error.message }, error.status);
    return response({ error: "internal_error" }, 500);
  }
}
export async function cleanup(env: Env) {
  const now = seconds();
  await env.DB.batch([
    q(env, "DELETE FROM host_challenges WHERE expires<=?", now),
    q(env, "DELETE FROM email_tokens WHERE expires<=?", now),
    q(env, "DELETE FROM oauth_states WHERE expires<=?", now),
    q(env, "DELETE FROM sessions WHERE expires<=?", now),
    q(env, "DELETE FROM device_auth WHERE expires<=?", now),
    q(env, "DELETE FROM rate_limits WHERE expires<=?", now),
    q(env, "DELETE FROM connection_requests WHERE expires<?", now - 86400),
  ]);
}
export default {
  fetch: (request: Request, env: Env) => handle(request, env),
  scheduled: (_event: ScheduledController, env: Env, ctx: ExecutionContext) =>
    ctx.waitUntil(cleanup(env)),
};
