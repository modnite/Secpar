import { passwordRoutes, accountPage } from "./passwords";
import { SignJWT, importJWK, calculateJwkThumbprint, type JWK } from "jose";

export interface Env {
  DB: D1Database;
  PUBLIC_ORIGIN: string;
  GITHUB_CLIENT_ID: string;
  GITHUB_CLIENT_SECRET: string;
  GRANT_PRIVATE_JWK: string;
  RESEND_API_KEY?: string;
  EMAIL_FROM?: string;
}
type Account = { id: string; handle: string; name: string };
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
const SESSION = "__Host-secpar-session";
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
function cookie(request: Request, name: string): string | undefined {
  return request.headers
    .get("cookie")
    ?.split(";")
    .map((s) => s.trim())
    .find((s) => s.startsWith(name + "="))
    ?.slice(name.length + 1);
}
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
function html(text: string, status = 200): Response {
  return new Response(text, {
    status,
    headers: {
      "content-type": "text/html; charset=utf-8",
      "cache-control": "no-store",
      "content-security-policy":
        "default-src 'none'; form-action 'self'; base-uri 'none'; frame-ancestors 'none'",
      "x-content-type-options": "nosniff",
    },
  });
}
const escapeHtml = (s: string) =>
  s
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;");
function setCookie(name: string, value: string, age: number): string {
  return `${name}=${value}; Path=/; HttpOnly; Secure; SameSite=Lax; Max-Age=${age}`;
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
  if (
    new URL(request.url).pathname === "/activate" &&
    request.headers
      .get("content-type")
      ?.startsWith("application/x-www-form-urlencoded")
  ) {
    const out: Record<string, string> = Object.create(null);
    new URLSearchParams(decoded).forEach((v, k) => {
      if (Object.hasOwn(out, k)) deny(400, "duplicate_form_field");
      out[k] = v;
    });
    return out;
  }
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
  const bearer = request.headers
    .get("authorization")
    ?.match(/^Bearer ([a-f0-9]{64})$/)?.[1];
  const token = bearer ?? cookie(request, SESSION);
  if (!token || !/^[a-f0-9]{64}$/.test(token)) deny(401, "sign_in_required");
  if (
    !bearer &&
    !["GET", "HEAD"].includes(request.method) &&
    request.headers.get("origin") !== origin(env)
  )
    deny(403, "invalid_origin");
  const row = await q(
    env,
    "SELECT a.id,a.handle,a.name FROM sessions s JOIN accounts a ON a.id=s.user_id WHERE s.hash=? AND s.expires>?",
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
    if (path === "/auth/start" && method === "GET")
      return new Response(null, {
        status: 302,
        headers: {
          location:
            base +
            "/auth/sign-in" +
            (url.searchParams.has("activate_code")
              ? "?activate_code=" +
                encodeURIComponent(url.searchParams.get("activate_code")!)
              : ""),
          "cache-control": "no-store",
        },
      });
    if (path === "/auth/github" && method === "GET") {
      await limit(request, env, "login", 10);
      if (
        !env.GITHUB_CLIENT_ID ||
        !env.GITHUB_CLIENT_SECRET ||
        env.GITHUB_CLIENT_ID.startsWith("CONFIGURE")
      )
        deny(503, "oauth_not_configured");
      const state = random(),
        verifier = random();
      const returnCode = url.searchParams.get("activate_code");
      await q(
        env,
        "INSERT INTO oauth_states(hash,verifier,return_code,expires) VALUES(?,?,?,?)",
        await hash(state),
        verifier,
        returnCode && /^[A-Z2-9]{8}$/.test(returnCode) ? returnCode : null,
        seconds() + 600,
      ).run();
      const challenge = btoa(
        String.fromCharCode(
          ...new Uint8Array(
            await crypto.subtle.digest(
              "SHA-256",
              new TextEncoder().encode(verifier),
            ),
          ),
        ),
      )
        .replaceAll("+", "-")
        .replaceAll("/", "_")
        .replaceAll("=", "");
      const github = new URL("https://github.com/login/oauth/authorize");
      for (const [k, v] of Object.entries({
        client_id: env.GITHUB_CLIENT_ID,
        redirect_uri: base + "/auth/callback",
        state,
        scope: "read:user",
        code_challenge: challenge,
        code_challenge_method: "S256",
      }))
        github.searchParams.set(k, v);
      return new Response(null, {
        status: 302,
        headers: {
          location: github.toString(),
          "set-cookie": setCookie(STATE, state, 600),
          "cache-control": "no-store",
        },
      });
    }
    if (path === "/auth/callback" && method === "GET") {
      await limit(request, env, "callback", 20);
      const state = url.searchParams.get("state"),
        code = url.searchParams.get("code");
      if (
        !state ||
        !code ||
        state.length !== 64 ||
        state !== cookie(request, STATE) ||
        code.length > 256
      )
        deny(400, "invalid_oauth_state");
      const row = await q(
        env,
        "DELETE FROM oauth_states WHERE hash=? AND expires>? RETURNING verifier,return_code",
        await hash(state),
        seconds(),
      ).first<{ verifier: string; return_code: string | null }>();
      if (!row) deny(400, "oauth_state_expired_or_used");
      const tokenResponse = await providerFetch(
        "https://github.com/login/oauth/access_token",
        {
          method: "POST",
          signal: AbortSignal.timeout(10000),
          headers: {
            accept: "application/json",
            "content-type": "application/json",
          },
          body: JSON.stringify({
            client_id: env.GITHUB_CLIENT_ID,
            client_secret: env.GITHUB_CLIENT_SECRET,
            code,
            redirect_uri: base + "/auth/callback",
            code_verifier: row.verifier,
          }),
        },
      );
      if (!tokenResponse.ok) deny(502, "oauth_exchange_failed");
      const tokenData = (await tokenResponse.json()) as {
        access_token?: string;
      };
      if (!tokenData.access_token) deny(401, "oauth_refused");
      const profileResponse = await providerFetch(
        "https://api.github.com/user",
        {
          signal: AbortSignal.timeout(10000),
          headers: {
            authorization: `Bearer ${tokenData.access_token}`,
            accept: "application/vnd.github+json",
            "user-agent": "Secpar",
          },
        },
      );
      if (!profileResponse.ok) deny(502, "profile_lookup_failed");
      const profile = (await profileResponse.json()) as {
        id?: number;
        login?: string;
        name?: string;
      };
      if (!Number.isSafeInteger(profile.id) || !profile.login)
        deny(502, "invalid_profile");
      await q(
        env,
        "INSERT INTO accounts(id,github_id,handle,name) VALUES(?,?,?,?) ON CONFLICT(github_id) DO UPDATE SET handle=excluded.handle,name=excluded.name",
        crypto.randomUUID(),
        String(profile.id),
        profile.login.slice(0, 80),
        (profile.name || profile.login).slice(0, 80),
      ).run();
      const account = await q(
        env,
        "SELECT id FROM accounts WHERE github_id=?",
        String(profile.id),
      ).first<{ id: string }>();
      if (!account) deny(500, "account_creation_failed");
      const session = random();
      await q(
        env,
        "INSERT INTO sessions(hash,user_id,expires) VALUES(?,?,?)",
        await hash(session),
        account.id,
        seconds() + 30 * 86400,
      ).run();
      const headers = new Headers({
        location:
          base +
          (row.return_code ? "/activate?code=" + row.return_code : "/v1/me"),
        "cache-control": "no-store",
      });
      headers.append("set-cookie", setCookie(STATE, "", 0));
      headers.append("set-cookie", setCookie(SESSION, session, 30 * 86400));
      return new Response(null, { status: 302, headers });
    }
    if (path === "/auth/device" && method === "POST") {
      await limit(request, env, "device", 10);
      const b = await body(request);
      const name = text(b.name, 60);
      const secret = random();
      const alphabet = "ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
      const bytes = crypto.getRandomValues(new Uint8Array(8));
      const code = Array.from(bytes, (b) => alphabet[b % 32]).join("");
      await q(
        env,
        "INSERT INTO device_auth(hash,code,name,expires) VALUES(?,?,?,?)",
        await hash(secret),
        code,
        name,
        seconds() + 600,
      ).run();
      return response(
        {
          device_code: secret,
          user_code: code,
          verification_uri: base + "/activate",
          verification_uri_complete: base + "/activate?code=" + code,
          expires_in: 600,
          interval: 5,
        },
        201,
      );
    }
    if (path === "/auth/device/token" && method === "POST") {
      await limit(request, env, "device_poll", 120);
      const b = await body(request),
        secret = text(b.device_code, 64);
      if (!/^[a-f0-9]{64}$/.test(secret)) deny(400, "invalid_device_code");
      const digest = await hash(secret);
      const row = await q(
        env,
        "SELECT * FROM device_auth WHERE hash=?",
        digest,
      ).first<{
        account: string | null;
        expires: number;
        poll_after: number;
        used: number;
      }>();
      if (!row || row.used || row.expires <= seconds())
        deny(400, "device_code_expired_or_used");
      if (row.poll_after > seconds()) deny(429, "slow_down");
      await q(
        env,
        "UPDATE device_auth SET poll_after=? WHERE hash=?",
        seconds() + 5,
        digest,
      ).run();
      if (!row.account) deny(400, "authorization_pending");
      const consumed = await q(
        env,
        "UPDATE device_auth SET used=1 WHERE hash=? AND used=0 AND expires>? RETURNING account",
        digest,
        seconds(),
      ).first<{ account: string }>();
      if (!consumed) deny(400, "device_code_expired_or_used");
      const token = random();
      await q(
        env,
        "INSERT INTO sessions(hash,user_id,expires) VALUES(?,?,?)",
        await hash(token),
        consumed.account,
        seconds() + 30 * 86400,
      ).run();
      return response({
        access_token: token,
        token_type: "Bearer",
        expires_in: 30 * 86400,
      });
    }
    if (path === "/activate" && method === "GET") {
      const code = url.searchParams.get("code") ?? "";
      if (!/^[A-Z2-9]{8}$/.test(code)) deny(400, "invalid_user_code");
      let signed: Account;
      try {
        signed = await user(request, env);
      } catch (error) {
        if (error instanceof ApiError && error.status === 401)
          return new Response(null, {
            status: 302,
            headers: {
              location: base + "/auth/start?activate_code=" + code,
              "cache-control": "no-store",
            },
          });
        throw error;
      }
      const row = await q(
        env,
        "SELECT name FROM device_auth WHERE code=? AND expires>? AND account IS NULL AND used=0",
        code,
        seconds(),
      ).first<{ name: string }>();
      if (!row) deny(400, "device_code_expired_or_used");
      return html(
        `<!doctype html><html lang="en"><meta charset="utf-8"><title>Secpar device approval</title><h1>Authorize ${escapeHtml(row.name)}</h1><p>Signed in as ${escapeHtml(signed.handle)}.</p><p>Approve only if this code matches the one in your Secpar app: <strong>${code}</strong></p><form method="post" action="/activate"><input type="hidden" name="code" value="${code}"><button type="submit">Approve device</button></form></html>`,
      );
    }
    const me = await user(request, env);
    if (path === "/activate" && method === "POST") {
      const code = text((await body(request)).code, 8);
      if (!/^[A-Z2-9]{8}$/.test(code)) deny(400, "invalid_user_code");
      const changed = await q(
        env,
        "UPDATE device_auth SET account=? WHERE code=? AND account IS NULL AND used=0 AND expires>?",
        me.id,
        code,
        seconds(),
      ).run();
      if (!changed.meta.changes) deny(409, "device_code_expired_or_used");
      return html(
        '<!doctype html><html lang="en"><meta charset="utf-8"><title>Secpar</title><h1>Device approved</h1><p>Return to Secpar.</p></html>',
      );
    }

    if (path === "/account" && method === "GET") return accountPage(me.handle);
    if (path === "/v1/me" && method === "GET") return response(me);
    if (path === "/auth/logout" && method === "POST") {
      const token =
        request.headers
          .get("authorization")
          ?.match(/^Bearer ([a-f0-9]{64})$/)?.[1] ??
        cookie(request, SESSION) ??
        "";
      await q(
        env,
        "DELETE FROM sessions WHERE hash=?",
        await hash(token),
      ).run();
      return new Response(null, {
        status: request.headers.get("accept")?.includes("text/html")
          ? 303
          : 204,
        headers: {
          ...(request.headers.get("accept")?.includes("text/html")
            ? { location: "/auth/sign-in" }
            : {}),
          "set-cookie": setCookie(SESSION, "", 0),
          "cache-control": "no-store",
        },
      });
    }
    if (path === "/v1/friends" && method === "GET") {
      const rows = await q(
        env,
        "SELECT x.id,x.handle,x.name,f.state,f.requester FROM friends f JOIN accounts x ON x.id=CASE WHEN f.a=? THEN f.b ELSE f.a END WHERE (f.a=? OR f.b=?) AND NOT EXISTS(SELECT 1 FROM blocks z WHERE (z.owner=? AND z.blocked=x.id) OR (z.owner=x.id AND z.blocked=?))",
        me.id,
        me.id,
        me.id,
        me.id,
        me.id,
      ).all();
      return response(rows.results);
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
      const id = crypto.randomUUID();
      await q(
        env,
        "INSERT INTO connection_requests(id,computer,owner,guest,requested,state,expires) VALUES(?,?,?,?,?,'pending',?)",
        id,
        computer,
        host.owner,
        me.id,
        JSON.stringify(requested),
        seconds() + 300,
      ).run();
      return response({ id, state: "pending", expires_in: 300 }, 201);
    }
    if (path === "/v1/requests" && method === "GET")
      return response(
        (
          await q(
            env,
            "SELECT * FROM connection_requests WHERE (owner=? OR guest=?) AND expires>? ORDER BY expires",
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
        return response({ grant: token, expires_at: expires });
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
