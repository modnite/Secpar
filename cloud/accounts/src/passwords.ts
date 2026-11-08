import { scrypt, timingSafeEqual, randomBytes } from "node:crypto";
import { APP_ACCOUNT_NOTE, escapeHtml, field, heading, renderPage } from "./ui";

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
  if (typeof value !== "string" || [...value].length < 8 || value.length > 256)
    fail(400, "Use a password of 8 to 256 characters.");
  return value;
}
function email(value: unknown): string {
  if (typeof value !== "string") fail(400, "Enter a valid email address.");
  const normalized = value.trim().toLowerCase();
  if (normalized.length > 254 || !/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(normalized))
    fail(400, "Enter a valid email address.");
  return normalized;
}
// no-referrer makes browser form POSTs send Origin: null.
// same-origin preserves Origin without leaking recovery URLs off-site.
function page(title: string, content: string, status = 200): Response {
  return renderPage(title, content, status, { "referrer-policy": "same-origin" });
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
      fail(429, "Too many attempts. Wait a few minutes and try again.");
  }
}
function mailReady(env: PasswordEnv) {
  if (!env.RESEND_API_KEY || !env.EMAIL_FROM)
    fail(
      503,
      "Email sending is not set up yet, so sign-up and password reset are closed for now.",
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
    purpose === "verify" ? "Confirm your email" : "Reset your password";
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
      text: `${action}:\n${link}\n\nThis link works for 30 minutes. It is for your Secpar app account. If you did not ask for it, ignore this email.`,
    }),
  });
  if (!result.ok)
    fail(
      503,
      "The email could not be sent. Try again in a few minutes.",
    );
}
export function accountPage(handle: string): Response {
  return page(
    "Secpar app account",
    `${heading("Secpar app account", APP_ACCOUNT_NOTE)}<p>This browser is signed in as <strong>${escapeHtml(handle)}</strong>. The Secpar app keeps its own sign-in.</p><form method="post" action="/auth/logout"><button class="button outline" type="submit">Sign out of this browser</button></form><p class="links"><a href="/auth/forgot">Reset password</a></p>`,
  );
}
const inputs = field(
  "email",
  "Email",
  'type="email" name="email" autocomplete="email" required maxlength="254"',
);
const passwordInput = (newPassword: boolean) =>
  field(
    "password",
    newPassword ? "New password" : "Password",
    `type="password" name="password" autocomplete="${newPassword ? "new-password" : "current-password"}" required minlength="8" maxlength="256"`,
    newPassword ? "At least 8 characters." : undefined,
  );
const submit = (label: string) =>
  `<button class="button" type="submit">${escapeHtml(label)}</button>`;
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
      const linking = !!code && /^[A-Z2-9]{8}$/.test(code);
      const hidden = linking
        ? `<input type="hidden" name="activate_code" value="${code}">`
        : "";
      const carry = linking ? "?activate_code=" + code : "";
      if (path === "/auth/sign-in")
        return page(
          "Secpar app account",
          `${heading("Secpar app account", APP_ACCOUNT_NOTE)}${linking ? '<p class="notice">Sign in to approve the computer that is waiting in the Secpar app.</p>' : ""}<form method="post">${hidden}${inputs}${passwordInput(false)}${submit("Sign in")}</form><p class="links"><a href="/auth/register${carry}">Create an account</a><a href="/auth/forgot">Reset password</a></p>`,
        );
      if (path === "/auth/register") {
        mailReady(env);
        return page(
          "Create an app account",
          `${heading("Create an app account", APP_ACCOUNT_NOTE)}<form method="post">${hidden}${inputs}${field("handle", "Username", 'name="handle" autocomplete="username" pattern="[a-zA-Z0-9_]{3,30}" required minlength="3" maxlength="30"', "3 to 30 letters, numbers or underscores. Friends see it.")}${passwordInput(true)}${submit("Create account")}</form><p class="fine">We email you a link to confirm the address. It works for 30 minutes.</p><p class="links"><a href="/auth/sign-in${carry}">I already have an account</a></p>`,
        );
      }
      if (path === "/auth/forgot") {
        mailReady(env);
        return page(
          "Reset your password",
          `${heading("Reset your password", "Enter the email of your app account. We send a link that works for 30 minutes.")}<form method="post">${inputs}${submit("Send reset link")}</form><p class="links"><a href="/auth/sign-in">Back to sign in</a></p>`,
        );
      }
      const token = url.searchParams.get("token");
      if (!token || !/^[a-f0-9]{64}$/.test(token))
        fail(400, "This link is not valid. Ask for a new one.");
      const verifying = path.endsWith("verify");
      return page(
        verifying ? "Confirm your email" : "Choose a new password",
        `${heading(verifying ? "Confirm your email" : "Choose a new password", verifying ? "Select the button to confirm this address for your Secpar app account." : "Saving a new password signs your account out on every device.")}<form method="post"><input type="hidden" name="token" value="${token}">${verifying ? "" : passwordInput(true)}${submit(verifying ? "Confirm email" : "Save password")}</form>`,
      );
    }
    if (request.method !== "POST") fail(405, "Method not allowed.");
    if (request.headers.get("origin") !== new URL(env.PUBLIC_ORIGIN).origin)
      fail(403, "The request came from another site and was refused.");
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
          fail(400, "Use a username of 3 to 30 letters, numbers or underscores.");
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
        `${heading("Check your email")}<p>If the address can receive mail, a link is on its way. It works for 30 minutes. Look in the spam folder if it does not arrive.</p><p>Open the link, then sign in again in the Secpar app.</p><p class="links"><a href="/auth/sign-in">Back to sign in</a></p>`,
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
          "The email or password is wrong, or the email is not confirmed yet.",
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
        fail(401, "Your password changed while you signed in. Sign in again.");
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
      fail(400, "This link is not valid or has expired. Ask for a new one.");
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
    if (!results[0].meta.changes)
      fail(400, "This link is not valid or has expired. Ask for a new one.");
    const done = purpose === "verify" ? "Email confirmed" : "Password changed";
    return page(
      done,
      `${heading(done)}<p>${purpose === "verify" ? "Open the Secpar app and sign in with this email and your password." : "Every device was signed out. Open the Secpar app and sign in with the new password."}</p><a class="button outline" href="/auth/sign-in">Sign in on this page</a>`,
    );
  } catch (error) {
    if (error instanceof Failure)
      return page(
        "Cannot continue",
        `${heading("Cannot continue")}<p class="notice" role="alert">${escapeHtml(error.message)}</p><p class="links"><a href="/auth/sign-in">Back to sign in</a></p>`,
        error.status,
      );
    return page(
      "Cannot continue",
      `${heading("Cannot continue")}<p class="notice" role="alert">Something went wrong on our side. Try again in a few minutes.</p><p class="links"><a href="/auth/sign-in">Back to sign in</a></p>`,
      500,
    );
  }
}
