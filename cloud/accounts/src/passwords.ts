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
const escape = (s: string) =>
  s
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;");
class Failure extends Error {
  constructor(
    readonly status: number,
    message: string,
  ) {
    super(message);
  }
}
function fail(status: number, message: string): never {
  throw new Failure(status, message);
}
const sessionCookie = (token: string) =>
  `__Host-secpar-session=${token}; Path=/; Secure; HttpOnly; SameSite=Lax; Max-Age=${30 * 86400}`;

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
  if (typeof value !== "string" || [...value].length < 15 || value.length > 256)
    fail(400, "Use a password of 15 to 256 characters.");
  return value;
}
function email(value: unknown): string {
  if (typeof value !== "string") fail(400, "Enter a valid email address.");
  const normalized = value.trim().toLowerCase();
  if (normalized.length > 254 || !/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(normalized))
    fail(400, "Enter a valid email address.");
  return normalized;
}
function page(title: string, content: string, status = 200): Response {
  const nonce = random();
  return new Response(
    `<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>${escape(title)} · Secpar</title><style nonce="${nonce}">:root{color-scheme:light dark;font:17px system-ui}body{max-width:28rem;margin:3rem auto;padding:0 1.2rem}input,button{box-sizing:border-box;font:inherit;padding:.7rem;width:100%;margin:.4rem 0 1rem}label{display:block}a{color:inherit}h1{font-size:1.7rem}</style><h1>${escape(title)}</h1>${content}</html>`,
    {
      status,
      headers: {
        "content-type": "text/html; charset=utf-8",
        "cache-control": "no-store",
        "referrer-policy": "no-referrer",
        "x-content-type-options": "nosniff",
        "content-security-policy": `default-src 'none'; style-src 'nonce-${nonce}'; form-action 'self'; base-uri 'none'; frame-ancestors 'none'`,
      },
    },
  );
}
async function data(request: Request): Promise<Record<string, unknown>> {
  const reader = request.body?.getReader();
  if (!reader) return fail(400, "A request body is required.");
  const parts: Uint8Array[] = [];
  let length = 0;
  while (true) {
    const chunk = await reader.read();
    if (chunk.done) break;
    length += chunk.value.length;
    if (length > 8192) {
      await reader.cancel();
      fail(413, "Request too large.");
    }
    parts.push(chunk.value);
  }
  const bytes = new Uint8Array(length);
  let offset = 0;
  for (const part of parts) {
    bytes.set(part, offset);
    offset += part.length;
  }
  const raw = new TextDecoder().decode(bytes);
  if (
    request.headers
      .get("content-type")
      ?.startsWith("application/x-www-form-urlencoded")
  ) {
    const out: Record<string, unknown> = Object.create(null);
    for (const [k, v] of new URLSearchParams(raw)) {
      if (Object.hasOwn(out, k)) fail(400, "Duplicate form field.");
      out[k] = v;
    }
    return out;
  }
  try {
    const out = JSON.parse(raw);
    if (!out || typeof out !== "object" || Array.isArray(out))
      fail(400, "Invalid request.");
    return out;
  } catch {
    return fail(400, "Invalid request.");
  }
}
async function rate(
  env: PasswordEnv,
  request: Request,
  action: string,
  identity = "",
) {
  // Per IP plus per email account, shared across the Worker's instances.
  for (const [scope, maximum] of [
    [request.headers.get("cf-connecting-ip") ?? "local", 10],
    ...(identity ? [[identity, 5]] : []),
  ] as [string, number][]) {
    const row = await q(
      env,
      "INSERT INTO rate_limits(key,count,expires) VALUES(?,1,?) ON CONFLICT(key) DO UPDATE SET count=count+1 RETURNING count",
      `password:${action}:${Math.floor(now() / 600)}:${await digest(scope)}`,
      now() + 1200,
    ).first<{ count: number }>();
    if (!row || row.count > maximum)
      fail(429, "Too many attempts. Please try again later.");
  }
}
function mailReady(env: PasswordEnv) {
  if (!env.RESEND_API_KEY || !env.EMAIL_FROM)
    fail(
      503,
      "Email delivery is being configured. Registration and password reset are not available yet.",
    );
}
async function send(
  env: PasswordEnv,
  to: string,
  purpose: "verify" | "reset",
  token: string,
  sendFetch: typeof fetch,
) {
  const link = `${env.PUBLIC_ORIGIN}/auth/${purpose}?token=${token}`;
  const action =
    purpose === "verify" ? "Verify your email" : "Reset your password";
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
      subject: `${action} · Secpar`,
      text: `${action}:\n${link}\n\nThis link expires in 30 minutes. If you did not request this, ignore this email.`,
    }),
  });
  if (!result.ok)
    fail(
      503,
      "Email delivery is temporarily unavailable. Please try again later.",
    );
}
export function accountPage(handle: string): Response {
  return page(
    "Secpar account",
    `<p>Signed in as <strong>${escape(handle)}</strong>.</p><p><a href="/auth/forgot">Reset password</a></p><form method="post" action="/auth/logout"><button>Sign out</button></form>`,
  );
}
const inputs =
  '<label>Email<input type="email" name="email" autocomplete="email" required maxlength="254"></label>';
const passwordInput = (newPassword: boolean) =>
  `<label>Password<input type="password" name="password" autocomplete="${newPassword ? "new-password" : "current-password"}" required minlength="15" maxlength="256"></label>`;
export async function passwordRoutes(
  request: Request,
  env: PasswordEnv,
  sendFetch: typeof fetch = fetch,
): Promise<Response | null> {
  const url = new URL(request.url),
    path = url.pathname;
  if (
    ![
      "/auth/sign-in",
      "/auth/register",
      "/auth/forgot",
      "/auth/verify",
      "/auth/reset",
    ].includes(path)
  )
    return null;
  try {
    if (request.method === "GET") {
      const code = url.searchParams.get("activate_code");
      const hidden =
        code && /^[A-Z2-9]{8}$/.test(code)
          ? `<input type="hidden" name="activate_code" value="${code}">`
          : "";
      if (path === "/auth/sign-in")
        return page(
          "Sign in to Secpar",
          `<form method="post">${hidden}${inputs}${passwordInput(false)}<button>Sign in</button></form><p><a href="/auth/register${hidden ? "?activate_code=" + code : ""}">Create account</a> · <a href="/auth/forgot">Forgot password?</a></p>`,
        );
      if (path === "/auth/register") {
        mailReady(env);
        return page(
          "Create your account",
          `<form method="post">${hidden}${inputs}<label>Username<input name="handle" autocomplete="username" pattern="[a-zA-Z0-9_]{3,30}" required minlength="3" maxlength="30"></label>${passwordInput(true)}<p>Use at least 15 characters.</p><button>Create account</button></form>`,
        );
      }
      if (path === "/auth/forgot") {
        mailReady(env);
        return page(
          "Reset your password",
          `<form method="post">${inputs}<button>Send reset link</button></form>`,
        );
      }
      const token = url.searchParams.get("token");
      if (!token || !/^[a-f0-9]{64}$/.test(token)) fail(400, "Invalid link.");
      return page(
        path.endsWith("verify") ? "Verify your email" : "Choose a new password",
        `<form method="post"><input type="hidden" name="token" value="${token}">${path.endsWith("reset") ? passwordInput(true) : ""}<button>${path.endsWith("verify") ? "Verify email" : "Reset password"}</button></form>`,
      );
    }
    if (request.method !== "POST") fail(405, "Method not allowed.");
    if (request.headers.get("origin") !== new URL(env.PUBLIC_ORIGIN).origin)
      fail(403, "Invalid request origin.");
    const b = await data(request);
    if (path === "/auth/register" || path === "/auth/forgot") {
      mailReady(env);
      const address = email(b.email);
      await rate(env, request, path, address);
      let account = await q(
        env,
        "SELECT user_id FROM password_accounts WHERE email=?",
        address,
      ).first<{ user_id: string }>();
      if (path === "/auth/register") {
        const pass = password(b.password);
        if (
          typeof b.handle !== "string" ||
          !/^[a-zA-Z0-9_]{3,30}$/.test(b.handle)
        )
          fail(400, "Use a username of 3–30 letters, numbers or underscores.");
        const encoded = await passwordHash(pass); // Also performed for an existing email.
        if (!account) {
          const id = crypto.randomUUID();
          try {
            await env.DB.batch([
              q(
                env,
                "INSERT INTO accounts(id,github_id,handle,name) VALUES(?,?,?,?)",
                id,
                `password:${id}`,
                b.handle,
                b.handle,
              ),
              q(
                env,
                "INSERT INTO password_accounts(user_id,email,password_hash,created) VALUES(?,?,?,?)",
                id,
                address,
                encoded,
                now(),
              ),
            ]);
            account = { user_id: id };
          } catch {
            // A competing signup may have won the unique email constraint.
            const existing = await q(
              env,
              "SELECT user_id FROM password_accounts WHERE email=?",
              address,
            ).first();
            if (!existing) throw new Error("Account creation failed");
          }
        } else account = null; // Do not resend another person's signup verification.
      }
      if (account) {
        const token = random(),
          purpose = path === "/auth/register" ? "verify" : "reset";
        const tokenHash = await digest(token);
        await q(
          env,
          "INSERT INTO email_tokens(hash,user_id,purpose,expires) VALUES(?,?,?,?)",
          tokenHash,
          account.user_id,
          purpose,
          now() + 1800,
        ).run();
        try {
          await send(env, address, purpose, token, sendFetch);
        } catch (error) {
          await q(
            env,
            "DELETE FROM email_tokens WHERE hash=?",
            tokenHash,
          ).run();
          throw error;
        }
      }
      return page(
        "Check your email",
        '<p>If this address can receive the requested link, an email is on its way. You can request a password reset if you already have an account.</p><a href="/auth/sign-in">Back to sign in</a>',
      );
    }
    if (path === "/auth/sign-in") {
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
      const match = await passwordMatches(
        pass,
        account?.password_hash ?? dummy,
      );
      if (!account || !match || !account.verified)
        fail(
          401,
          "Email or password is incorrect, or the email has not been verified.",
        );
      const token = random();
      const created = await q(
        env,
        "INSERT INTO sessions(hash,user_id,expires) SELECT ?,user_id,? FROM password_accounts WHERE user_id=? AND password_hash=? AND verified=1",
        await digest(token),
        now() + 30 * 86400,
        account.user_id,
        account.password_hash,
      ).run();
      if (!created.meta.changes)
        fail(401, "Your credentials changed. Please sign in again.");
      const code =
        typeof b.activate_code === "string" &&
        /^[A-Z2-9]{8}$/.test(b.activate_code)
          ? b.activate_code
          : null;
      return new Response(null, {
        status: 303,
        headers: {
          location: code ? `/activate?code=${code}` : "/account",
          "set-cookie": sessionCookie(token),
          "cache-control": "no-store",
        },
      });
    }
    await rate(env, request, "email-token");
    const token = b.token;
    if (typeof token !== "string" || !/^[a-f0-9]{64}$/.test(token))
      fail(400, "Invalid or expired link.");
    const purpose = path === "/auth/verify" ? "verify" : "reset";
    // KDF finishes before the atomic batch so a hashing failure cannot consume a link.
    const encoded =
      purpose === "reset" ? await passwordHash(password(b.password)) : null;
    const tokenHash = await digest(token);
    // D1 batches are transactions. Every update is bound to the still-live token;
    // deletion is last, so a competing replay cannot update credentials or sessions.
    const select =
      "SELECT user_id FROM email_tokens WHERE hash=? AND purpose=? AND expires>?";
    const results = await env.DB.batch([
      encoded
        ? q(
            env,
            `UPDATE password_accounts SET password_hash=?,verified=1 WHERE user_id IN (${select})`,
            encoded,
            tokenHash,
            purpose,
            now(),
          )
        : q(
            env,
            `UPDATE password_accounts SET verified=1 WHERE user_id IN (${select})`,
            tokenHash,
            purpose,
            now(),
          ),
      ...(encoded
        ? [
            q(
              env,
              `DELETE FROM sessions WHERE user_id IN (${select})`,
              tokenHash,
              purpose,
              now(),
            ),
            q(
              env,
              `DELETE FROM email_tokens WHERE user_id IN (${select}) AND hash<>?`,
              tokenHash,
              purpose,
              now(),
              tokenHash,
            ),
          ]
        : []),
      q(
        env,
        "DELETE FROM email_tokens WHERE hash=? AND purpose=? AND expires>?",
        tokenHash,
        purpose,
        now(),
      ),
    ]);
    if (!results[0].meta.changes) fail(400, "Invalid or expired link.");
    return page(
      purpose === "verify" ? "Email verified" : "Password reset",
      '<p>You can now sign in.</p><a href="/auth/sign-in">Sign in</a>',
    );
  } catch (error) {
    if (error instanceof Failure)
      return page(
        "Unable to continue",
        `<p>${escape(error.message)}</p><a href="/auth/sign-in">Back to sign in</a>`,
        error.status,
      );
    return page(
      "Unable to continue",
      "<p>Something went wrong. Please try again later.</p>",
      500,
    );
  }
}
