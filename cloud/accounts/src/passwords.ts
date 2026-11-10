// Sign up, sign in and password recovery for the apps. Every route takes and returns JSON and needs no
// browser. Email carries a six digit code. A session is a bearer token of 64 hex characters, stored hashed.
import { scrypt, timingSafeEqual, randomBytes } from "node:crypto";

export interface PasswordEnv {
  DB: D1Database;
  PUBLIC_ORIGIN: string;
  RESEND_API_KEY?: string;
  EMAIL_FROM?: string;
}
const now = () => Math.floor(Date.now() / 1000);
const random = () => randomBytes(32).toString("hex");
const q = (env: PasswordEnv, sql: string, ...args: (string | number)[]) =>
  env.DB.prepare(sql).bind(...args);
const digest = async (s: string) =>
  Buffer.from(
    await crypto.subtle.digest("SHA-256", new TextEncoder().encode(s)),
  ).toString("hex");
class Failure extends Error {
  constructor(
    readonly status: number,
    message: string,
    readonly code: string,
  ) {
    super(message);
  }
}
function fail(status: number, message: string, code: string): never {
  throw new Failure(status, message, code);
}
const CODE_SECONDS = 600;

// OWASP's 16 MiB scrypt configuration. Native crypto avoids a JS/WASM KDF.
function derive(password: string, salt: string): Promise<Buffer> {
  return new Promise((resolve, reject) =>
    scrypt(
      password,
      salt,
      32,
      { N: 16384, r: 8, p: 5, maxmem: 32 * 1024 * 1024 },
      (error, value) => (error ? reject(error) : resolve(value)),
    ),
  );
}
export async function passwordHash(password: string): Promise<string> {
  const salt = random();
  return `scrypt-16384-8-5$${salt}$${(await derive(password, salt)).toString("hex")}`;
}
export async function passwordMatches(
  password: string,
  encoded: string,
): Promise<boolean> {
  const [scheme, salt, expected, extra] = encoded.split("$");
  if (
    scheme !== "scrypt-16384-8-5" ||
    !/^[a-f0-9]{64}$/.test(salt ?? "") ||
    !/^[a-f0-9]{64}$/.test(expected ?? "") ||
    extra !== undefined
  )
    return false;
  return timingSafeEqual(
    await derive(password, salt),
    Buffer.from(expected, "hex"),
  );
}
function password(value: unknown): string {
  // Preserve whitespace and permit long passphrases; no composition rules.
  if (typeof value !== "string" || [...value].length < 8 || value.length > 256)
    fail(400, "Use a password of 8 to 256 characters.", "invalid_password");
  return value;
}
function email(value: unknown): string {
  if (typeof value !== "string") fail(400, "Enter a valid email address.", "invalid_email");
  const normalized = value.trim().toLowerCase();
  if (normalized.length > 254 || !/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(normalized))
    fail(400, "Enter a valid email address.", "invalid_email");
  return normalized;
}
function codeOf(value: unknown): string {
  const code = typeof value === "string" ? value.replaceAll(/\s/g, "") : "";
  if (!/^[0-9]{6}$/.test(code)) fail(400, "Enter the six digit code from the email.", "invalid_code");
  return code;
}
async function data(request: Request): Promise<Record<string, unknown>> {
  const reader = request.body?.getReader();
  if (!reader) return fail(400, "A request body is required.", "invalid_request");
  const parts: Uint8Array[] = [];
  let length = 0;
  while (true) {
    const chunk = await reader.read();
    if (chunk.done) break;
    length += chunk.value.length;
    if (length > 8192) {
      await reader.cancel();
      fail(413, "Request too large.", "request_too_large");
    }
    parts.push(chunk.value);
  }
  const bytes = new Uint8Array(length);
  let offset = 0;
  for (const part of parts) {
    bytes.set(part, offset);
    offset += part.length;
  }
  try {
    const out = JSON.parse(new TextDecoder().decode(bytes));
    if (!out || typeof out !== "object" || Array.isArray(out))
      fail(400, "Invalid request.", "invalid_request");
    return out;
  } catch (error) {
    if (error instanceof Failure) throw error;
    return fail(400, "Invalid request.", "invalid_request");
  }
}
/** Counts one try per address and per identity inside a window. Fixed windows, shared across instances. */
async function rate(
  env: PasswordEnv,
  request: Request,
  action: string,
  identity = "",
  limits: { window: number; ip: number; id: number } = { window: 600, ip: 10, id: 5 },
) {
  for (const [scope, maximum] of [
    [request.headers.get("cf-connecting-ip") ?? "local", limits.ip],
    ...(identity ? [[identity, limits.id]] : []),
  ] as [string, number][]) {
    const row = await q(
      env,
      "INSERT INTO rate_limits(key,count,expires) VALUES(?,1,?) ON CONFLICT(key) DO UPDATE SET count=count+1 RETURNING count",
      `password:${action}:${Math.floor(now() / limits.window)}:${await digest(scope)}`,
      now() + limits.window * 2,
    ).first<{ count: number }>();
    if (!row || row.count > maximum)
      fail(429, "Too many attempts. Wait a few minutes and try again.", "rate_limited");
  }
}
function mailReady(env: PasswordEnv) {
  if (!env.RESEND_API_KEY || !env.EMAIL_FROM)
    fail(
      503,
      "Email sending is not set up yet, so sign-up and password reset are closed for now.",
      "mail_unavailable",
    );
}
function newCode(): string {
  // Uniform over 000000 to 999999: reject the biased tail of the 32 bit range.
  const limit = Math.floor(2 ** 32 / 1_000_000) * 1_000_000;
  const buffer = new Uint32Array(1);
  do crypto.getRandomValues(buffer);
  while (buffer[0] >= limit);
  return String(buffer[0] % 1_000_000).padStart(6, "0");
}
const codeHash = (user: string, purpose: string, code: string) => digest(`${user}:${purpose}:${code}`);
async function send(
  env: PasswordEnv,
  to: string,
  purpose: "verify" | "reset",
  code: string,
  sendFetch: typeof fetch,
) {
  const action = purpose === "verify" ? "Confirm your email" : "Reset your password";
  const result = await sendFetch("https://api.resend.com/emails", {
    method: "POST",
    signal: AbortSignal.timeout(10000),
    headers: {
      authorization: `Bearer ${env.RESEND_API_KEY}`,
      "content-type": "application/json",
    },
    body: JSON.stringify({
      from: env.EMAIL_FROM,
      to: [to],
      subject: `${code} is your Secpar code`,
      text: `${action} in the Secpar app with this code:\n\n${code}\n\nIt works for 10 minutes. If you did not ask for it, ignore this email.`,
    }),
  });
  if (!result.ok)
    fail(503, "The email could not be sent. Try again in a few minutes.", "mail_failed");
}
/** Replaces any earlier code of this kind, stores the new one hashed and emails it. */
async function issue(env: PasswordEnv, to: string, user: string, purpose: "verify" | "reset", sendFetch: typeof fetch) {
  const code = newCode(),
    hash = await codeHash(user, purpose, code);
  await env.DB.batch([
    q(env, "DELETE FROM email_tokens WHERE user_id=? AND purpose=?", user, purpose),
    q(env, "INSERT INTO email_tokens(hash,user_id,purpose,expires) VALUES(?,?,?,?)", hash, user, purpose, now() + CODE_SECONDS),
  ]);
  try {
    await send(env, to, purpose, code, sendFetch);
  } catch (error) {
    await q(env, "DELETE FROM email_tokens WHERE hash=?", hash).run();
    throw error;
  }
}
const json = (value: unknown, status = 200) =>
  new Response(JSON.stringify(value), {
    status,
    headers: { "content-type": "application/json", "cache-control": "no-store", "x-content-type-options": "nosniff" },
  });
const sent = () => json({ sent: true }, 202);
const MAIL_LIMIT = { window: 3600, ip: 20, id: 3 };

export async function passwordRoutes(
  request: Request,
  env: PasswordEnv,
  sendFetch: typeof fetch = fetch,
): Promise<Response | null> {
  const path = new URL(request.url).pathname;
  if (!["register", "resend", "verify", "sign-in", "forgot", "reset"].some((r) => path === `/v1/app/${r}`))
    return null;
  try {
    if (request.method !== "POST") fail(405, "Method not allowed.", "method_not_allowed");
    const b = await data(request);
    const route = path.slice("/v1/app/".length);
    if (route === "register" || route === "forgot" || route === "resend") {
      mailReady(env);
      const address = email(b.email);
      await rate(env, request, route, address);
      let account = await q(
        env,
        "SELECT user_id,verified FROM password_accounts WHERE email=?",
        address,
      ).first<{ user_id: string; verified: number }>();
      if (route === "register") {
        const pass = password(b.password);
        if (typeof b.handle !== "string" || !/^[a-zA-Z0-9_]{3,30}$/.test(b.handle))
          fail(400, "Use a username of 3 to 30 letters, numbers or underscores. Capitals do not matter.", "invalid_username");
        const lower = b.handle.toLowerCase();
        // The answer depends only on the username, so it says nothing about any email.
        if (await q(env, "SELECT 1 FROM accounts WHERE handle_lower=?", lower).first())
          fail(409, "That username is taken. Choose another.", "username_taken");
        const encoded = await passwordHash(pass); // Also performed for an existing email.
        if (!account) {
          const id = crypto.randomUUID();
          try {
            await env.DB.batch([
              q(env, "INSERT INTO accounts(id,github_id,handle,handle_lower,name) VALUES(?,?,?,?,?)", id, `password:${id}`, b.handle, lower, b.handle),
              q(env, "INSERT INTO password_accounts(user_id,email,password_hash,created) VALUES(?,?,?,?)", id, address, encoded, now()),
            ]);
            account = { user_id: id, verified: 0 };
          } catch {
            // A competing signup may have won the unique email constraint or the username.
            if (await q(env, "SELECT 1 FROM accounts WHERE handle_lower=?", lower).first())
              fail(409, "That username is taken. Choose another.", "username_taken");
            if (!(await q(env, "SELECT 1 FROM password_accounts WHERE email=?", address).first()))
              throw new Error("Account creation failed");
            account = null;
          }
        } else account = null; // Never mail a code for somebody else's signup.
      } else if (route === "resend" && account?.verified) account = null;
      if (account) {
        await rate(env, request, "mail", address, MAIL_LIMIT);
        await issue(env, address, account.user_id, route === "forgot" ? "reset" : "verify", sendFetch);
      }
      return sent();
    }
    if (route === "sign-in") {
      const address = email(b.email);
      const pass = password(b.password);
      await rate(env, request, "login", address);
      const account = await q(
        env,
        "SELECT user_id,password_hash,verified FROM password_accounts WHERE email=?",
        address,
      ).first<{ user_id: string; password_hash: string; verified: number }>();
      // Use the same work factor even for missing accounts.
      const dummy = `scrypt-16384-8-5$${"0".repeat(64)}$${"0".repeat(64)}`;
      const match = await passwordMatches(pass, account?.password_hash ?? dummy);
      if (!account || !match || !account.verified)
        fail(401, "The email or password is wrong, or the email is not confirmed yet.", "bad_credentials");
      const token = random(),
        expires = now() + 30 * 86400;
      const created = await q(
        env,
        "INSERT INTO sessions(hash,user_id,expires) SELECT ?,user_id,? FROM password_accounts WHERE user_id=? AND password_hash=? AND verified=1",
        await digest(token),
        expires,
        account.user_id,
        account.password_hash,
      ).run();
      if (!created.meta.changes)
        fail(401, "Your password changed while you signed in. Sign in again.", "bad_credentials");
      const me = await q(env, "SELECT id,handle FROM accounts WHERE id=?", account.user_id).first<{ id: string; handle: string }>();
      return json({ token, expires_at: expires, user: me });
    }
    // verify and reset: an email, the code from it, and for reset the new password.
    const address = email(b.email);
    const code = codeOf(b.code);
    const purpose = route === "verify" ? "verify" : "reset";
    await rate(env, request, "code", address);
    // The KDF finishes before the batch so a hashing failure cannot use up the code.
    const encoded = purpose === "reset" ? await passwordHash(password(b.password)) : null;
    const account = await q(env, "SELECT user_id FROM password_accounts WHERE email=?", address).first<{ user_id: string }>();
    const bad = () => fail(400, "That code is wrong or has expired. Ask for a new one.", "invalid_code");
    if (!account) return bad();
    const hash = await codeHash(account.user_id, purpose, code);
    // D1 batches are transactions. Every update is bound to the still-live code and deletion is last,
    // so a competing replay cannot change credentials or sessions.
    const select = "SELECT user_id FROM email_tokens WHERE hash=? AND purpose=? AND expires>?";
    const results = await env.DB.batch([
      encoded
        ? q(env, `UPDATE password_accounts SET password_hash=?,verified=1 WHERE user_id IN (${select})`, encoded, hash, purpose, now())
        : q(env, `UPDATE password_accounts SET verified=1 WHERE user_id IN (${select})`, hash, purpose, now()),
      ...(encoded ? [q(env, `DELETE FROM sessions WHERE user_id IN (${select})`, hash, purpose, now())] : []),
      q(env, "DELETE FROM email_tokens WHERE hash=? AND purpose=? AND expires>?", hash, purpose, now()),
    ]);
    if (!results[0].meta.changes) return bad();
    return json({ done: true });
  } catch (error) {
    if (error instanceof Failure)
      return json({ error: error.code, message: error.message }, error.status);
    return json({ error: "server_error", message: "Something went wrong on our side. Try again in a few minutes." }, 500);
  }
}
