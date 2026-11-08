import { hostRoutes } from "./hosts";
import { passwordRoutes, accountPage } from "./passwords";
import { assetRoute, escapeHtml, heading, renderPage } from "./ui";
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
const html = (title: string, content: string, status = 200): Response =>
  renderPage(title, content, status);
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
async function friendSettings(env: Env, owner: string, friend: string) {
  const row = await q(
    env,
    "SELECT input,connect FROM friend_settings WHERE owner=? AND friend=?",
    owner,
    friend,
  ).first<{ input: string; connect: string }>();
  return { input: row?.input ?? "view", connect: row?.connect ?? "ask" };
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
// GitHub logins may contain hyphens. A hyphen becomes an underscore.
function githubUsername(login: string): string {
  let name = login.replaceAll(/[^a-zA-Z0-9]/g, "_").slice(0, 30);
  if (name.length < 3) name = name.padEnd(3, "_");
  return name;
}
function withSuffix(name: string, n: number): string {
  if (n < 2) return name;
  const tail = "_" + n;
  return name.slice(0, 30 - tail.length) + tail;
}
// The username is chosen once, when the account is created. Later logins keep it.
async function upsertGithubAccount(
  env: Env,
  githubId: string,
  login: string,
  fullName: string | undefined,
) {
  const name = (fullName || login).slice(0, 80);
  const changed = await q(
    env,
    "UPDATE accounts SET name=? WHERE github_id=?",
    name,
    githubId,
  ).run();
  if (changed.meta.changes) return;
  const base = githubUsername(login);
  for (let n = 1; n <= 40; n++) {
    const id = crypto.randomUUID();
    const handle =
      n < 40 ? withSuffix(base, n) : base.slice(0, 21) + "_" + id.slice(0, 8);
    try {
      await q(
        env,
        "INSERT INTO accounts(id,github_id,handle,handle_lower,name) VALUES(?,?,?,?,?)",
        id,
        githubId,
        handle,
        handle.toLowerCase(),
        name,
      ).run();
      return;
    } catch {
      // The name is taken, or a parallel login created this account first.
      if (await q(env, "SELECT 1 FROM accounts WHERE github_id=?", githubId).first())
        return;
    }
  }
  deny(500, "account_creation_failed");
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
    const asset = assetRoute(request);
    if (asset) return asset;
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
      await upsertGithubAccount(env, String(profile.id), profile.login, profile.name);
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
        "Approve a computer",
        `${heading("Approve " + row.name, "The Secpar app on that device asked to sign in to your app account.")}<p>This browser is signed in as <strong>${escapeHtml(signed.handle)}</strong>.</p><p>The app shows this code. Approve only if the codes match:</p><p class="code" aria-label="Device code">${code}</p><form method="post" action="/activate"><input type="hidden" name="code" value="${code}"><button class="button" type="submit">Approve</button></form><p class="fine">The code works for 10 minutes after the app shows it. If you did not start this, close the page.</p>`,
      );
    }
    const hostApi = {body,fail:deny,hash,limit:(r:Request,_env:unknown,operation:string,maximum:number)=>limit(r,env,operation,maximum)};
    const machineResponse=await hostRoutes(request,env,null,hostApi);
    if(machineResponse)return machineResponse;
    const me = await user(request, env);
    const ownerHostResponse=await hostRoutes(request,env,me,hostApi);
    if(ownerHostResponse)return ownerHostResponse;
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
        "Device approved",
        `${heading("Computer approved")}<p>Go back to the Secpar app. It signs in within a few seconds. You can close this page.</p>`,
      );
    }

    if (path === "/account" && method === "GET") return accountPage(me.handle);
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
        "SELECT x.id,x.handle,x.name,f.state,f.requester,COALESCE(s.input,'view') AS input,COALESCE(s.connect,'ask') AS connect FROM friends f JOIN accounts x ON x.id=CASE WHEN f.a=? THEN f.b ELSE f.a END LEFT JOIN friend_settings s ON s.owner=? AND s.friend=x.id WHERE (f.a=? OR f.b=?) AND NOT EXISTS(SELECT 1 FROM blocks z WHERE (z.owner=? AND z.blocked=x.id) OR (z.owner=x.id AND z.blocked=?))",
        me.id,
        me.id,
        me.id,
        me.id,
        me.id,
        me.id,
      ).all();
      return response(rows.results);
    }
    const settings = path.match(/^\/v1\/friends\/([^/]+)\/settings$/);
    if (settings && ["GET", "PUT"].includes(method)) {
      const other = settings[1];
      if (!(await friends(env, me.id, other))) deny(404, "account_unavailable");
      if (method === "PUT") {
        const b = await body(request);
        const input = b.input ?? null,
          connect = b.connect ?? null;
        if (
          (input !== null && input !== "view" && input !== "full") ||
          (connect !== null && connect !== "ask" && connect !== "always") ||
          (input === null && connect === null)
        )
          deny(400, "invalid_value");
        await q(
          env,
          "INSERT INTO friend_settings(owner,friend,input,connect) VALUES(?,?,COALESCE(?,'view'),COALESCE(?,'ask')) ON CONFLICT(owner,friend) DO UPDATE SET input=COALESCE(?,input),connect=COALESCE(?,connect)",
          me.id,
          other,
          input,
          connect,
          input,
          connect,
        ).run();
        if (input === "view")
          // A view only friend cannot keep a grant that already allows input.
          await q(
            env,
            "UPDATE connection_requests SET state='revoked' WHERE owner=? AND guest=? AND state IN ('pending','approved','claimed') AND (requested LIKE '%\"keyboard\"%' OR requested LIKE '%\"mouse\"%') AND (COALESCE(approved,requested) LIKE '%\"keyboard\"%' OR COALESCE(approved,requested) LIKE '%\"mouse\"%')",
            me.id,
            other,
          ).run();
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
      const allowed =
        policy.input === "full"
          ? requested
          : requested.filter((p) => p !== "keyboard" && p !== "mouse");
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
        if (
          (await friendSettings(env, row.owner, row.guest)).input !== "full" &&
          approved.some((p) => p === "keyboard" || p === "mouse")
        )
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
