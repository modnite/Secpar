import { beforeEach, describe, expect, it } from "vitest";
import { DatabaseSync } from "node:sqlite";
import { readFileSync } from "node:fs";
import { generateKeyPair, exportJWK, importJWK, jwtVerify, SignJWT, importPKCS8, importX509, compactVerify } from "jose";
import { handle, hash, type Env } from "../src/index";

class Statement {
  constructor(
    readonly db: DatabaseSync,
    readonly sql: string,
    readonly values: (string | number | null)[] = [],
  ) {}
  bind(...values: (string | number | null)[]) {
    return new Statement(this.db, this.sql, values);
  }
  async first() {
    return this.db.prepare(this.sql).get(...this.values) ?? null;
  }
  async all() {
    return { results: this.db.prepare(this.sql).all(...this.values) };
  }
  async run() {
    const result = this.db.prepare(this.sql).run(...this.values);
    return { success: true, meta: { changes: Number(result.changes) } };
  }
}
class Database {
  db = new DatabaseSync(":memory:");
  constructor() {
    this.db.exec(
      readFileSync(
        new URL("../migrations/0001_accounts.sql", import.meta.url),
        "utf8",
      ),
    );
    this.db.exec(
      readFileSync(
        new URL("../migrations/0002_passwords.sql", import.meta.url),
        "utf8",
      ),
    );
    this.db.exec(readFileSync(new URL("../migrations/0003_hosts.sql", import.meta.url), "utf8"));
    this.db.exec(readFileSync(new URL("../migrations/0004_owner_pairings.sql", import.meta.url), "utf8"));
    this.db.exec(readFileSync(new URL("../migrations/0005_usernames.sql", import.meta.url), "utf8"));
    this.db.exec(readFileSync(new URL("../migrations/0006_directory_and_friend_settings.sql", import.meta.url), "utf8"));
    this.db.exec(readFileSync(new URL("../migrations/0007_host_presence.sql", import.meta.url), "utf8"));
    this.db.exec(readFileSync(new URL("../migrations/0008_keyboard_and_mouse.sql", import.meta.url), "utf8"));
  }
  prepare(sql: string) {
    return new Statement(this.db, sql);
  }
  async batch(statements: Statement[]) {
    this.db.exec("BEGIN");
    try {
      const out = [];
      for (const s of statements) out.push(await s.run());
      this.db.exec("COMMIT");
      return out;
    } catch (e) {
      this.db.exec("ROLLBACK");
      throw e;
    }
  }
}
const base = "https://accounts.example.test";
let db: Database, env: Env;
const tokens = {
  alice: "a".repeat(64),
  bob: "b".repeat(64),
  mallory: "c".repeat(64),
};
async function call(
  path: string,
  actor: keyof typeof tokens | null = "alice",
  method = "GET",
  data?: unknown,
  extra: Record<string, string> = {},
) {
  const headers: Record<string, string> = { ...extra };
  if (actor) headers.authorization = `Bearer ${tokens[actor]}`;
  if (data !== undefined) headers["content-type"] = "application/json";
  return handle(
    new Request(base + path, {
      method,
      headers,
      body: data === undefined ? undefined : JSON.stringify(data),
    }),
    env,
  );
}
async function friendship() {
  expect((await call("/v1/friends/bob", "alice", "PUT")).status).toBe(202);
  expect((await call("/v1/friends/alice/accept", "bob", "POST")).status).toBe(
    200,
  );
}
async function computer() {
  const r = await call("/v1/computers", "alice", "POST", {
    name: "Desktop",
    platform: "windows",
  });
  expect(r.status).toBe(201);
  const id = (await r.json()).id as string;
  // A host that is online and sharing, so claims in these tests have somewhere to point.
  db.db
    .prepare("INSERT INTO host_keys(computer,thumbprint,certificate_fp,credential_hash,last_seen,sharing,pc,relay) VALUES(?,?,?,?,?,1,?,?)")
    .run(id, `thumb-${id}`, "ab".repeat(32), `cred-${id}`, Math.floor(Date.now() / 1000), "00000000-0000-4000-8000-0000000000c1", "https://relay.example");
  return id;
}
async function invitation(host: string) {
  const r = await call("/v1/requests", "bob", "POST", {
    computer: host,
    permissions: ["view", "audio", "gamepad"],
  });
  expect(r.status).toBe(201);
  return (await r.json()).id as string;
}
beforeEach(async () => {
  db = new Database();
  const { privateKey } = await generateKeyPair("ES256", { extractable: true });
  env = {
    DB: db as unknown as D1Database,
    PUBLIC_ORIGIN: base,
    GITHUB_CLIENT_ID: "test-client",
    GITHUB_CLIENT_SECRET: "test-secret",
    GRANT_PRIVATE_JWK: JSON.stringify(await exportJWK(privateKey)),
  };
  for (const [id, token] of Object.entries(tokens)) {
    await db
      .prepare("INSERT INTO accounts(id,github_id,handle,handle_lower,name) VALUES(?,?,?,?,?)")
      .bind(id, id, id, id, id)
      .run();
    await db
      .prepare("INSERT INTO sessions(hash,user_id,expires) VALUES(?,?,?)")
      .bind(await hash(token), id, Math.floor(Date.now() / 1000) + 3600)
      .run();
  }
});
describe("authentication", () => {
  it("requires authentication and origin protection for browser mutations", async () => {
    expect((await call("/v1/friends", null)).status).toBe(401);
    const request = new Request(base + "/v1/friends/bob", {
      method: "PUT",
      headers: {
        cookie: `__Host-secpar-session=${tokens.alice}`,
        origin: "https://evil.example",
      },
    });
    expect((await handle(request, env)).status).toBe(403);
    expect((await call("/auth/logout", "alice", "POST")).status).toBe(204);
    expect((await call("/v1/me")).status).toBe(401);
  });
  it("uses browser-bound one-use OAuth state and PKCE and stores only session hashes", async () => {
    const begin = await handle(new Request(base + "/auth/github"), env);
    const location = new URL(begin.headers.get("location")!);
    const state = location.searchParams.get("state")!;
    expect(location.searchParams.get("code_challenge_method")).toBe("S256");
    expect(location.searchParams.get("code_challenge")).toHaveLength(43);
    let verified = false;
    const provider: typeof fetch = async (input, init) => {
      if (String(input).includes("access_token")) {
        const body = JSON.parse(String(init?.body));
        expect(body.code_verifier).toHaveLength(64);
        expect(body.redirect_uri).toBe(base + "/auth/callback");
        verified = true;
        return Response.json({ access_token: "provider-only-secret" });
      }
      return Response.json({
        id: 12345,
        login: "newfriend",
        name: "New Friend",
      });
    };
    const callback = new Request(
      base + `/auth/callback?code=provider-code&state=${state}`,
      { headers: { cookie: `__Host-secpar-oauth=${state}` } },
    );
    expect(
      (await handle(new Request(callback.url), env, provider)).status,
    ).toBe(400);
    const result = await handle(callback, env, provider);
    expect(result.status).toBe(302);
    expect(verified).toBe(true);
    const cookie = result.headers
      .getSetCookie()
      .find((s) => s.startsWith("__Host-secpar-session="))!;
    expect(cookie).toContain("HttpOnly");
    expect(cookie).toContain("Secure");
    const token = cookie.split(";")[0].split("=")[1];
    expect(
      await db
        .prepare("SELECT 1 FROM sessions WHERE hash=?")
        .bind(token)
        .first(),
    ).toBeNull();
    const me = await handle(
      new Request(base + "/v1/me", {
        headers: { cookie: cookie.split(";")[0] },
      }),
      env,
    );
    expect((await me.json()).handle).toBe("newfriend");
    expect(
      (
        await handle(
          new Request(callback.url, {
            headers: { cookie: `__Host-secpar-oauth=${state}` },
          }),
          env,
          provider,
        )
      ).status,
    ).toBe(400);
  });
});
describe("friend and guest boundaries", () => {
  it("a friend request grants no access and cannot be self-accepted", async () => {
    const host = await computer();
    expect((await call("/v1/friends/bob", "alice", "PUT")).status).toBe(202);
    expect((await call("/v1/friends/bob/accept", "alice", "POST")).status).toBe(
      404,
    );
    expect(
      (
        await call("/v1/requests", "bob", "POST", {
          computer: host,
          permissions: ["view"],
        })
      ).status,
    ).toBe(403);
  });
  it("only the owner approves and only the intended guest claims one limited signed grant", async () => {
    await friendship();
    const host = await computer();
    const id = await invitation(host);
    expect(
      (
        await call(`/v1/requests/${id}/approve`, "bob", "POST", {
          permissions: ["view"],
        })
      ).status,
    ).toBe(403);
    expect(
      (await call(`/v1/requests/${id}/claim`, "mallory", "POST")).status,
    ).toBe(404);
    expect(
      (
        await call(`/v1/requests/${id}/approve`, "alice", "POST", {
          permissions: ["view", "keyboard"],
        })
      ).status,
    ).toBe(400);
    expect(
      (
        await call(`/v1/requests/${id}/approve`, "alice", "POST", {
          permissions: ["view", "gamepad"],
        })
      ).status,
    ).toBe(200);
    const claims = await Promise.all([
      call(`/v1/requests/${id}/claim`, "bob", "POST"),
      call(`/v1/requests/${id}/claim`, "bob", "POST"),
    ]);
    expect(claims.map((r) => r.status).sort()).toEqual([200, 409]);
    const token = (await claims.find((r) => r.status === 200)!.json()).grant;
    const keys = await (
      await handle(new Request(base + "/.well-known/jwks.json"), env)
    ).json();
    expect(keys.keys[0].d).toBeUndefined();
    const { payload } = await jwtVerify(
      token,
      await importJWK(keys.keys[0], "ES256"),
      { issuer: base, audience: host },
    );
    expect(payload.sub).toBe("bob");
    expect(payload.jti).toBe(id);
    expect(payload.permissions).toEqual(["gamepad", "view"]);
    expect(payload.exp! - payload.iat!).toBeLessThanOrEqual(60);
    await expect(
      jwtVerify(token, await importJWK(keys.keys[0], "ES256"), {
        issuer: base,
        audience: "different-host",
      }),
    ).rejects.toThrow();
  });
  it("blocking revokes claimed access and prevents new invitations in both directions", async () => {
    await friendship();
    const host = await computer();
    const id = await invitation(host);
    await call(`/v1/requests/${id}/approve`, "alice", "POST", {
      permissions: ["view"],
    });
    await call(`/v1/requests/${id}/claim`, "bob", "POST");
    expect(
      (await (await call(`/v1/requests/${id}`, "bob")).json()).active,
    ).toBe(true);
    await call("/v1/blocks/alice", "bob", "PUT");
    expect(
      (await (await call(`/v1/requests/${id}`, "alice")).json()).active,
    ).toBe(false);
    expect((await call("/v1/friends/alice", "bob", "PUT")).status).toBe(404);
    expect((await call("/v1/friends/bob", "alice", "PUT")).status).toBe(404);
    expect(
      (
        await call("/v1/requests", "bob", "POST", {
          computer: host,
          permissions: ["view"],
        })
      ).status,
    ).toBe(403);
  });
  it("does not issue admin permissions or expired grants", async () => {
    await friendship();
    const host = await computer();
    expect(
      (
        await call("/v1/requests", "bob", "POST", {
          computer: host,
          permissions: ["unlock"],
        })
      ).status,
    ).toBe(400);
    const id = await invitation(host);
    await db
      .prepare("UPDATE connection_requests SET expires=0 WHERE id=?")
      .bind(id)
      .run();
    expect(
      (
        await call(`/v1/requests/${id}/approve`, "alice", "POST", {
          permissions: ["view"],
        })
      ).status,
    ).toBe(409);
    expect((await call(`/v1/requests/${id}/claim`, "bob", "POST")).status).toBe(
      403,
    );
  });
});

describe("native device sign-in", () => {
  it("keeps polling credentials off the browser and exchanges an approved code once", async () => {
    const start = await call("/auth/device", null, "POST", {
      name: "Windows desktop",
    });
    expect(start.status).toBe(201);
    const device = await start.json();
    expect(device.verification_uri_complete).not.toContain(device.device_code);
    const login = await handle(
      new Request(device.verification_uri_complete),
      env,
    );
    expect(login.status).toBe(302);
    expect(login.headers.get("location")).toContain(
      "/auth/start?activate_code=",
    );
    expect(
      (
        await call("/auth/device/token", null, "POST", {
          device_code: device.device_code,
        })
      ).status,
    ).toBe(400);
    expect(
      (await call("/activate", "alice", "POST", { code: device.user_code }))
        .status,
    ).toBe(200);
    expect(
      (await call("/activate", "bob", "POST", { code: device.user_code }))
        .status,
    ).toBe(409);
    await db.prepare("UPDATE device_auth SET poll_after=0").run();
    const tokenResult = await call("/auth/device/token", null, "POST", {
      device_code: device.device_code,
    });
    expect(tokenResult.status).toBe(200);
    const token = (await tokenResult.json()).access_token;
    const me = await handle(
      new Request(base + "/v1/me", {
        headers: { authorization: `Bearer ${token}` },
      }),
      env,
    );
    expect((await me.json()).id).toBe("alice");
    expect(
      (
        await call("/auth/device/token", null, "POST", {
          device_code: device.device_code,
        })
      ).status,
    ).toBe(400);
  });
});

describe("configuration and resource limits", () => {
  it("does not consume an approved claim while signing is unconfigured", async () => {
    await friendship();
    const host = await computer();
    const id = await invitation(host);
    await call(`/v1/requests/${id}/approve`, "alice", "POST", {
      permissions: ["view"],
    });
    const key = env.GRANT_PRIVATE_JWK;
    env.GRANT_PRIVATE_JWK = "";
    expect((await call(`/v1/requests/${id}/claim`, "bob", "POST")).status).toBe(
      503,
    );
    env.GRANT_PRIVATE_JWK = key;
    expect((await call(`/v1/requests/${id}/claim`, "bob", "POST")).status).toBe(
      200,
    );
  });
  it("limits unauthenticated login creation and bounds streamed request bodies", async () => {
    for (let n = 0; n < 10; n++)
      expect(
        (await handle(new Request(base + "/auth/github"), env)).status,
      ).toBe(302);
    expect((await handle(new Request(base + "/auth/github"), env)).status).toBe(
      429,
    );
    const request = new Request(base + "/v1/computers", {
      method: "POST",
      headers: {
        authorization: `Bearer ${tokens.alice}`,
        "content-type": "application/json",
        "content-length": "1",
      },
      body: JSON.stringify({ name: "x".repeat(9000), platform: "windows" }),
    });
    expect((await handle(request, env)).status).toBe(413);
  });
});

describe("email and password accounts", () => {
  const pass = "correct horse battery staple";
  const nextPass = "a different long password phrase";
  async function form(
    path: string,
    fields: Record<string, string>,
    send?: typeof fetch,
    requestOrigin = base,
  ) {
    return handle(
      new Request(base + path, {
        method: "POST",
        headers: {
          origin: requestOrigin,
          "content-type": "application/x-www-form-urlencoded",
        },
        body: new URLSearchParams(fields),
      }),
      env,
      send,
    );
  }
  function mailCapture(messages: { to: string; text: string }[]): typeof fetch {
    return async (_input, init) => {
      const message = JSON.parse(String(init?.body));
      expect(message.text).not.toContain(pass);
      messages.push({ to: message.to[0], text: message.text });
      return Response.json({ id: "email-id" });
    };
  }
  function link(message: { text: string }) {
    return new URL(message.text.split("\n")[1]);
  }
  it("keeps registration and reset closed without configured email delivery", async () => {
    expect((await call("/auth/register", null)).status).toBe(503);
    expect(
      (
        await form("/auth/register", {
          email: "friend@example.com",
          handle: "friend",
          password: pass,
        })
      ).status,
    ).toBe(503);
    expect(
      (await form("/auth/forgot", { email: "friend@example.com" })).status,
    ).toBe(503);
    expect(
      db.db.prepare("SELECT count(*) AS n FROM password_accounts").get()?.n,
    ).toBe(0);
  });
  it("verifies email, signs in, resets once and revokes old sessions", async () => {
    env.RESEND_API_KEY = "test-mail-key";
    env.EMAIL_FROM = "Secpar <accounts@secpar.win>";
    const messages: { to: string; text: string }[] = [];
    const send = mailCapture(messages);
    const fields = {
      email: "Friend@Example.com",
      handle: "friend",
      password: pass,
    };
    expect((await form("/auth/register", fields, send)).status).toBe(200);
    expect(messages).toHaveLength(1);
    expect(messages[0].to).toBe("friend@example.com");
    const stored = db.db.prepare("SELECT * FROM password_accounts").get()!;
    expect(stored.password_hash).not.toContain(pass);
    expect(stored.password_hash).toMatch(/^scrypt-16384-8-5/);
    expect((await form("/auth/sign-in", fields)).status).toBe(401);
    const verify = link(messages[0]);
    const token = verify.searchParams.get("token")!;
    expect(db.db.prepare("SELECT hash FROM email_tokens").get()?.hash).not.toBe(
      token,
    );
    // Mail scanners cannot consume the link with GET.
    expect((await handle(new Request(verify), env)).status).toBe(200);
    expect(
      (await form("/auth/verify", { token }, send, "https://evil.test")).status,
    ).toBe(403);
    expect((await form("/auth/verify", { token })).status).toBe(200);
    expect((await form("/auth/verify", { token })).status).toBe(400);
    const login = await form("/auth/sign-in", {
      ...fields,
      activate_code: "ABCDEFGH",
    });
    expect(login.status).toBe(303);
    expect(login.headers.get("location")).toBe("/activate?code=ABCDEFGH");
    const cookie = login.headers.get("set-cookie")!.split(";")[0];
    expect(login.headers.get("set-cookie")).toContain(
      "Secure; HttpOnly; SameSite=Lax",
    );
    expect(
      (await call("/v1/me", null, "GET", undefined, { cookie })).status,
    ).toBe(200);
    // Existing signup cannot overwrite an account's credentials.
    expect(
      (await form("/auth/register", { ...fields, handle: "other_name", password: nextPass }, send))
        .status,
    ).toBe(200);
    expect(messages).toHaveLength(1);
    expect(
      (await form("/auth/forgot", { email: fields.email }, send)).status,
    ).toBe(200);
    const resetToken = link(messages[1]).searchParams.get("token")!;
    const results = await Promise.all([
      form("/auth/reset", { token: resetToken, password: nextPass }),
      form("/auth/reset", { token: resetToken, password: nextPass }),
    ]);
    expect(results.map((r) => r.status).sort()).toEqual([200, 400]);
    expect(
      (await call("/v1/me", null, "GET", undefined, { cookie })).status,
    ).toBe(401);
    expect((await form("/auth/sign-in", fields)).status).toBe(401);
    // Use another IP to avoid the intended 5-attempt account bucket in this test.
    db.db.exec("DELETE FROM rate_limits");
    expect(
      (await form("/auth/sign-in", { ...fields, password: nextPass })).status,
    ).toBe(303);
  });
  it("bounds input, requires origin, rejects weak passwords and limits login attempts", async () => {
    expect(
      (
        await form(
          "/auth/sign-in",
          { email: "x@example.com", password: pass },
          undefined,
          "https://evil.test",
        )
      ).status,
    ).toBe(403);
    expect(
      (
        await form("/auth/sign-in", {
          email: "x@example.com",
          password: "short",
        })
      ).status,
    ).toBe(400);
    for (let i = 0; i < 5; i++)
      expect(
        (
          await form("/auth/sign-in", {
            email: "missing@example.com",
            password: pass,
          })
        ).status,
      ).toBe(401);
    expect(
      (
        await form("/auth/sign-in", {
          email: "missing@example.com",
          password: pass,
        })
      ).status,
    ).toBe(429);
    const huge = new Request(base + "/auth/sign-in", {
      method: "POST",
      headers: { origin: base, "content-type": "application/json" },
      body: JSON.stringify({ password: "x".repeat(9000) }),
    });
    expect((await handle(huge, env)).status).toBe(413);
  });
  it("does not log in after expiry and does not change credentials with expired reset tokens", async () => {
    env.RESEND_API_KEY = "key";
    env.EMAIL_FROM = "accounts@secpar.win";
    const messages: { to: string; text: string }[] = [];
    await form(
      "/auth/register",
      { email: "new@example.com", handle: "friend", password: pass },
      mailCapture(messages),
    );
    const token = link(messages[0]).searchParams.get("token")!;
    db.db.exec("UPDATE email_tokens SET expires=0");
    expect((await form("/auth/verify", { token })).status).toBe(400);
    expect(
      (
        await form("/auth/sign-in", {
          email: "new@example.com",
          password: pass,
        })
      ).status,
    ).toBe(401);
  });
});

describe("browser form regression", () => {
  it("keeps the origin on form submissions and hides recovery URLs from other sites", async () => {
    env.RESEND_API_KEY = "test-key";
    env.EMAIL_FROM = "accounts@mail.secpar.win";
    for (const path of [
      "/auth/sign-in",
      "/auth/register",
      "/auth/forgot",
      "/auth/verify?token=" + "a".repeat(64),
      "/auth/reset?token=" + "b".repeat(64),
    ]) {
      const page = await handle(new Request(base + path), env);
      expect(page.status).toBe(200);
      // The browser sets Origin:null for form POSTs under no-referrer. Keep the
      // origin check; fix the policy that made legitimate forms fail it.
      expect(page.headers.get("referrer-policy")).toBe("same-origin");
      const markup = await page.text();
      if (
        path.includes("sign-in") ||
        path.includes("register") ||
        path.includes("reset")
      )
        expect(markup).toContain('minlength="8"');
    }
  });
  it("accepts an eight-character signup password and still rejects foreign origins", async () => {
    env.RESEND_API_KEY = "test-key";
    env.EMAIL_FROM = "accounts@mail.secpar.win";
    const form = {
      email: "eight@example.com",
      handle: "eight",
      password: "8char-ok",
    };
    const sent: string[] = [];
    const send: typeof fetch = async (_url, init) => {
      sent.push(String(init?.body));
      return Response.json({ id: "test" });
    };
    const submit = (origin: string) =>
      handle(
        new Request(base + "/auth/register", {
          method: "POST",
          headers: {
            origin,
            "content-type": "application/x-www-form-urlencoded",
          },
          body: new URLSearchParams(form),
        }),
        env,
        send,
      );
    expect((await submit("null")).status).toBe(403);
    expect((await submit("https://foreign.example")).status).toBe(403);
    expect((await submit(base)).status).toBe(200);
    expect(sent).toHaveLength(1);
    expect(
      db.db.prepare("SELECT count(*) AS n FROM password_accounts").get()?.n,
    ).toBe(1);
  });
});

describe("account pages", () => {
  const dash = String.fromCharCode(0x2014);
  it("states the purpose of the app account on the sign-in and sign-up pages", async () => {
    env.RESEND_API_KEY = "test-key";
    env.EMAIL_FROM = "accounts@mail.secpar.win";
    for (const path of ["/auth/sign-in", "/auth/register"]) {
      const markup = await (await handle(new Request(base + path), env)).text();
      expect(markup).toContain("only used inside the Secpar app");
      expect(markup).toContain("nothing to log in to on the website");
      expect(markup).toContain('rel="stylesheet" href="/assets/account.css"');
      expect(markup).toContain('<label for="email">');
      expect(markup).not.toContain(dash);
    }
  });
  it("keeps the native flow URLs and tells the user what the code is for", async () => {
    const start = await call("/auth/device", null, "POST", { name: "Desk PC" });
    const device = await start.json();
    const redirect = await handle(new Request(device.verification_uri_complete), env);
    const location = redirect.headers.get("location")!;
    expect(location).toContain("/auth/start?activate_code=");
    const code = device.user_code as string;
    const signIn = await handle(new Request(base + "/auth/sign-in?activate_code=" + code), env);
    const markup = await signIn.text();
    expect(markup).toContain('name="activate_code" value="' + code + '"');
    expect(markup).toContain("waiting in the Secpar app");
  });
  it("serves the stylesheet, fonts and wordmark from this origin only", async () => {
    for (const [path, type] of [
      ["/assets/account.css", "text/css"],
      ["/assets/rubik-400.woff", "font/woff"],
      ["/assets/rubik-500.woff", "font/woff"],
      ["/assets/secpar-wordmark.svg", "image/svg+xml"],
    ]) {
      const res = await handle(new Request(base + path), env);
      expect(res.status).toBe(200);
      expect(res.headers.get("content-type")).toContain(type);
      expect(res.headers.get("x-content-type-options")).toBe("nosniff");
    }
    expect((await handle(new Request(base + "/assets/other.css"), env)).status).toBe(401);
    expect((await handle(new Request(base + "/assets/account.css", { method: "POST" }), env)).status).toBe(405);
    const page = await handle(new Request(base + "/auth/sign-in"), env);
    expect(page.headers.get("content-security-policy")).toBe(
      "default-src 'none'; style-src 'self'; font-src 'self'; img-src 'self'; form-action 'self'; base-uri 'none'; frame-ancestors 'none'",
    );
  });
});

describe("blocked people", () => {
  it("lists only the current account's blocks and supports unblocking", async () => {
    expect((await call("/v1/blocks/bob", "alice", "PUT")).status).toBe(200);
    const blocked = await (await call("/v1/blocks")).json();
    expect(blocked.map((user: { id: string }) => user.id)).toEqual(["bob"]);
    expect(await (await call("/v1/blocks", "mallory")).json()).toEqual([]);
    expect((await call("/v1/blocks", null)).status).toBe(401);
    expect((await call("/v1/blocks/bob", "alice", "DELETE")).status).toBe(200);
    expect(await (await call("/v1/blocks")).json()).toEqual([]);
  });
});


// Disposable, publicly committed certificate/key. Never a production identity.
const hostCertificate=readFileSync(new URL("./fixtures/host-cert.pem",import.meta.url),"utf8");
const hostPrivate=readFileSync(new URL("./fixtures/host-key.pem",import.meta.url),"utf8");
async function hostProof(challenge:string,actor:string,changes:Record<string,unknown>={}, audience=base, typ="secpar-host-enrollment+jwt") {
  const encoded=hostCertificate.replace(/-----[^-]+-----/g,"").replace(/\s/g,"");
  const fp=Buffer.from(await crypto.subtle.digest("SHA-256",Buffer.from(encoded,"base64"))).toString("hex");
  return new SignJWT({purpose:"secpar-host-enrollment",challenge,tls_fingerprint:fp,name:"Fixture Windows",platform:"windows",...changes})
    .setProtectedHeader({alg:"ES256",typ}).setIssuer("secpar-host").setSubject(actor).setAudience(audience).setIssuedAt().setExpirationTime("60s").sign(await importPKCS8(hostPrivate,"ES256"));
}
async function startHost(actor:keyof typeof tokens="alice") {
  const response=await call("/v1/host-enrollment/start",actor,"POST");expect(response.status).toBe(201);
  return await response.json() as {challenge:string};
}
async function finishHost(challenge:string,actor:keyof typeof tokens="alice",proof?:string) {
  return call("/v1/host-enrollment/finish",actor,"POST",{certificate:hostCertificate,proof:proof??await hostProof(challenge,actor)});
}
async function machine(path:string,credential:string,method="GET",data?:unknown) {
  return call(path,null,method,data,{authorization:`Bearer ${credential}`});
}
async function enrolledHost() {
  const {challenge}=await startHost();const response=await finishHost(challenge);expect(response.status).toBe(201);
  return await response.json() as {computer:string;credential:string;certificate_fp:string};
}
describe("host enrollment and machine scope",()=>{
  it("verifies the public proof generated by the Rust host signer",async()=>{
    const vector=JSON.parse(readFileSync(new URL("./fixtures/rust-host-proof.json",import.meta.url),"utf8"));
    const verified=await compactVerify(vector.proof,await importX509(vector.certificate,"ES256"),{algorithms:["ES256"]});
    expect(verified.protectedHeader.typ).toBe("secpar-host-enrollment+jwt");
    const claims=JSON.parse(new TextDecoder().decode(verified.payload));
    expect(claims.aud).toBe("https://accounts.secpar.win");expect(claims.sub).toBe("interop-fixture");
    expect(claims.purpose).toBe("secpar-host-enrollment");expect(claims.exp-claims.iat).toBe(90);
  });

  it("requires private-key proof and an owner-bound, one-use challenge",async()=>{
    expect((await call("/v1/host-enrollment/start",null,"POST")).status).toBe(401);
    const {challenge}=await startHost();
    expect((await finishHost(challenge,"bob",await hostProof(challenge,"alice"))).status).toBe(403);
    expect((await finishHost(challenge,"alice",await hostProof(challenge,"alice",{tls_fingerprint:"0".repeat(64)}))).status).toBe(403);
    const results=await Promise.all([finishHost(challenge),finishHost(challenge)]);
    expect(results.map(r=>r.status).sort()).toEqual([201,409]);
    const linked=await results.find(r=>r.status===201)!.json();
    expect(linked.credential).toMatch(/^h1_[a-f0-9]{64}$/);
    const stored=db.db.prepare("SELECT * FROM host_keys").get()!;
    expect(stored.credential_hash).toBe(await hash(linked.credential));
    expect(stored.sharing).toBe(0);
    expect(db.db.prepare("SELECT count(*) AS n FROM computers").get()?.n).toBe(1);
    const reuse=await finishHost(challenge);expect(reuse.status).toBe(409);
  });
  it("expires challenges and rejects other audiences without burning a valid challenge",async()=>{
    const first=await startHost();
    const wrong=await hostProof(first.challenge,"alice",{},"https://other.invalid");
    expect((await finishHost(first.challenge,"alice",wrong)).status).toBe(403);
    const wrongType=await hostProof(first.challenge,"alice",{},base,"JWT");
    expect((await finishHost(first.challenge,"alice",wrongType)).status).toBe(403);
    const valid=await hostProof(first.challenge,"alice");
    const parts=valid.split(".");const signature=Buffer.from(parts[2],"base64url");signature[0]^=1;
    expect((await finishHost(first.challenge,"alice",parts.slice(0,2).join(".")+"."+signature.toString("base64url"))).status).toBe(403);
    expect((await finishHost(first.challenge)).status).toBe(201);
    const second=await startHost();db.db.exec("UPDATE host_challenges SET expires=0");
    expect((await finishHost(second.challenge)).status).toBe(409);
  });
  it("scopes machine credentials away from account, friendship and other hosts",async()=>{
    const host=await enrolledHost();
    expect((await machine("/v1/host/status",host.credential)).status).toBe(200);
    expect((await machine("/v1/host/heartbeat",host.credential,"POST")).status).toBe(200);
    expect((await machine("/v1/me",host.credential)).status).toBe(401);
    expect((await machine("/v1/friends",host.credential)).status).toBe(401);
    expect((await machine("/v1/host/not-allowed",host.credential)).status).toBe(403);
    expect((await call("/v1/host/status","alice")).status).toBe(401);
    const foreign=crypto.randomUUID();const request=crypto.randomUUID();
    db.db.prepare("INSERT INTO computers(id,owner,name,platform,created) VALUES(?,?,?,?,?)").run(foreign,"bob","Other PC","linux",1);
    db.db.prepare("INSERT INTO connection_requests(id,computer,owner,guest,requested,state,expires) VALUES(?,?,?,?,?,?,?)").run(request,foreign,"bob","alice",'["view"]',"pending",Math.floor(Date.now()/1000)+60);
    expect((await machine(`/v1/host/requests/${request}`,host.credential)).status).toBe(404);
    const other=await startHost("bob");
    expect((await finishHost(other.challenge,"bob")).status).toBe(409);
    expect((await call(`/v1/computers/${host.computer}/link`,"bob","DELETE")).status).toBe(404);
    expect((await call(`/v1/computers/${host.computer}/link`,"alice","DELETE")).status).toBe(200);
    expect((await machine("/v1/host/status",host.credential)).status).toBe(401);
  });
  it("rotates only machine credentials and revokes existing guest leases on relinking",async()=>{
    await friendship();const host=await enrolledHost();
    await machine("/v1/host/heartbeat",host.credential,"POST",{sharing:true,pc:"00000000-0000-4000-8000-0000000000c1",relay:"https://relay.example"});
    const request=await invitation(host.computer);
    await call(`/v1/requests/${request}/approve`,"alice","POST",{permissions:["view"]});
    await call(`/v1/requests/${request}/claim`,"bob","POST");
    expect((await (await machine(`/v1/host/requests/${request}`,host.credential)).json()).active).toBe(true);
    const {challenge}=await startHost();const next=await (await finishHost(challenge)).json();
    expect(next.computer).toBe(host.computer);expect(next.certificate_fp).toBe(host.certificate_fp);
    expect((await machine("/v1/host/status",host.credential)).status).toBe(401);
    expect((await (await machine(`/v1/host/requests/${request}`,next.credential)).json()).active).toBe(false);
    expect((await call("/v1/me","alice")).status).toBe(200);
  });
  it("keeps host discovery private and disabled until guest sharing is implemented",async()=>{
    await friendship();const host=await enrolledHost();
    await machine("/v1/host/heartbeat",host.credential,"POST");
    expect(await (await call("/v1/friend-computers","bob")).json()).toEqual([]);
    // Exercise the future discovery guard without exposing a production enable endpoint.
    db.db.exec("UPDATE host_keys SET sharing=1");
    expect((await (await call("/v1/friend-computers","bob")).json()).map((c:{id:string})=>c.id)).toEqual([host.computer]);
    expect(await (await call("/v1/friend-computers","mallory")).json()).toEqual([]);
    await call("/v1/blocks/bob","alice","PUT");
    expect(await (await call("/v1/friend-computers","bob")).json()).toEqual([]);
  });
});

describe("same-account owner pairing", () => {
  async function request(host: { computer: string }) {
    const r = await call("/v1/owner-pairings", "alice", "POST", { computer: host.computer, device: "Fixture laptop" });
    expect(r.status).toBe(201);
    return await r.json() as { id: string; expires: number };
  }
  function link(host: { certificate_fp: string }, token = "one-use-fixture") {
    return `secpar://pair?n=Fixture&h=192.168.1.2&p=48999&fp=${host.certificate_fp}&t=${token}`;
  }
  it("shows only owned machines and their actual enrollment/presence", async () => {
    const host = await enrolledHost();
    const offline = await (await call("/v1/computers", "alice")).json();
    expect(offline).toMatchObject([{ id: host.computer, linked: 1, online: 0, certificate_fp: host.certificate_fp }]);
    expect(JSON.stringify(offline)).not.toContain(host.credential);
    await machine("/v1/host/heartbeat", host.credential, "POST");
    expect((await (await call("/v1/computers", "alice")).json())[0].online).toBe(1);
    expect(await (await call("/v1/computers", "bob")).json()).toEqual([]);
    expect((await call("/v1/computers", null)).status).toBe(401);
  });
  it("round-trips an encrypted one-use result only to the requesting device session", async () => {
    const host = await enrolledHost(), pair = await request(host);
    expect((await call(`/v1/owner-pairings/${pair.id}`)).status).toBe(202);
    const jobs = await (await machine("/v1/host/owner-pairings", host.credential)).json();
    expect(jobs).toEqual([{ id: pair.id, device_name: "Fixture laptop", expires: pair.expires }]);
    expect((await machine(`/v1/host/owner-pairings/${pair.id}`, host.credential, "POST", { link: link(host) })).status).toBe(200);
    const stored = db.db.prepare("SELECT * FROM owner_pairings").get()!;
    expect(stored.sealed).toMatch(/^[\w-]+\.[\w-]+$/);
    expect(JSON.stringify(stored)).not.toContain("one-use-fixture");
    expect(JSON.stringify(stored)).not.toContain("secpar://");
    const otherSession = "e".repeat(64);
    db.db.prepare("INSERT INTO sessions(hash,user_id,expires) VALUES(?,?,?)").run(await hash(otherSession), "alice", pair.expires);
    expect((await call(`/v1/owner-pairings/${pair.id}`, null, "GET", undefined, { authorization: `Bearer ${otherSession}` })).status).toBe(404);
    expect((await call(`/v1/owner-pairings/${pair.id}`, "bob")).status).toBe(404);
    const delivered = await call(`/v1/owner-pairings/${pair.id}`);
    expect(delivered.status).toBe(200);
    expect(await delivered.json()).toMatchObject({ link: link(host), certificate_fp: host.certificate_fp });
    expect((await call(`/v1/owner-pairings/${pair.id}`)).status).toBe(404);
    expect(db.db.prepare("SELECT count(*) AS n FROM owner_pairings").get()?.n).toBe(0);
  });
  it("does not grant friendship or machine tokens owner authorization", async () => {
    await friendship(); const host = await enrolledHost(), pair = await request(host);
    expect((await call("/v1/owner-pairings", "bob", "POST", { computer: host.computer, device: "Other" })).status).toBe(404);
    expect((await machine("/v1/owner-pairings", host.credential, "POST", { computer: host.computer, device: "Other" })).status).toBe(401);
    expect((await call(`/v1/host/owner-pairings/${pair.id}`, "alice", "POST", { link: link(host) })).status).toBe(401);
    const second = "h1_" + "f".repeat(64), secondId = crypto.randomUUID();
    db.db.prepare("INSERT INTO computers(id,owner,name,platform,created) VALUES(?,?,?,?,?)").run(secondId, "alice", "Second owned PC", "linux", 1);
    db.db.prepare("INSERT INTO host_keys(computer,thumbprint,certificate_fp,credential_hash) VALUES(?,?,?,?)").run(secondId, "second-fixture", host.certificate_fp, await hash(second));
    expect(await (await machine("/v1/host/owner-pairings", second)).json()).toEqual([]);
    expect((await machine(`/v1/host/owner-pairings/${pair.id}`, second, "POST", { link: link(host) })).status).toBe(404);
  });
  it("rejects mismatched identities and malformed pairing links without burning the request", async () => {
    const host = await enrolledHost(), pair = await request(host);
    for (const bad of [link(host).replace(host.certificate_fp, "0".repeat(64)), link(host) + "&t=duplicate", link(host).replace("192.168.1.2", "https://evil.test"), link(host) + "#fragment", link(host).replace("secpar:", "https:")]) {
      expect((await machine(`/v1/host/owner-pairings/${pair.id}`, host.credential, "POST", { link: bad })).status).toBe(400);
    }
    expect((await machine(`/v1/host/owner-pairings/${pair.id}`, host.credential, "POST", { link: link(host) })).status).toBe(200);
    expect((await machine(`/v1/host/owner-pairings/${pair.id}`, host.credential, "POST", { link: link(host, "replacement") })).status).toBe(404);
    const results = await Promise.all([call(`/v1/owner-pairings/${pair.id}`), call(`/v1/owner-pairings/${pair.id}`)]);
    expect(results.map(r => r.status).sort()).toEqual([200, 404]);
  });
  it("bounds pending requests and invalidates them on expiry, cancellation, unlink and relink", async () => {
    const host = await enrolledHost(), pair = await request(host);
    for (let i = 0; i < 7; i++) await request(host);
    expect((await call("/v1/owner-pairings", "alice", "POST", { computer: host.computer, device: "Ninth" })).status).toBe(429);
    expect((await call(`/v1/owner-pairings/${pair.id}`, "bob", "DELETE")).status).toBe(404);
    expect((await call(`/v1/owner-pairings/${pair.id}`, "alice", "DELETE")).status).toBe(200);
    expect((await call(`/v1/owner-pairings/${pair.id}`)).status).toBe(404);
    const expiring = await request(host);
    db.db.prepare("UPDATE owner_pairings SET expires=0 WHERE id=?").run(expiring.id);
    expect((await call(`/v1/owner-pairings/${expiring.id}`)).status).toBe(404);
    expect((await machine(`/v1/host/owner-pairings/${expiring.id}`, host.credential, "POST", { link: link(host) })).status).toBe(404);
    const old = await request(host), challenge = await startHost();
    const next = await (await finishHost(challenge.challenge)).json();
    expect((await call(`/v1/owner-pairings/${old.id}`)).status).toBe(404);
    expect(await (await machine("/v1/host/owner-pairings", next.credential)).json()).toEqual([]);
    const revoked = await request(host);
    await machine("/v1/host/link", next.credential, "DELETE");
    expect((await call(`/v1/owner-pairings/${revoked.id}`)).status).toBe(404);
  });
  it("authenticates result ciphertext and request context", async () => {
    const host = await enrolledHost(), first = await request(host), second = await request(host);
    await machine(`/v1/host/owner-pairings/${first.id}`, host.credential, "POST", { link: link(host) });
    const sealed = db.db.prepare("SELECT sealed FROM owner_pairings WHERE id=?").get(first.id)?.sealed;
    db.db.prepare("UPDATE owner_pairings SET sealed=? WHERE id=?").run(sealed, second.id);
    expect((await call(`/v1/owner-pairings/${second.id}`)).status).toBe(409);
    expect((await call(`/v1/owner-pairings/${first.id}`)).status).toBe(200);
  });
  it("stops offering pairing work for a signed-out requester", async () => {
    const host = await enrolledHost(), pair = await request(host);
    expect((await call("/auth/logout", "alice", "POST")).status).toBe(204);
    expect(await (await machine("/v1/host/owner-pairings", host.credential)).json()).toEqual([]);
    expect((await machine(`/v1/host/owner-pairings/${pair.id}`, host.credential, "POST", { link: link(host) })).status).toBe(404);
  });
});

describe("usernames", () => {
  const add = (name: string, actor: keyof typeof tokens = "alice", method = "PUT") =>
    call("/v1/friends/by-username/" + name, actor, method);
  const pending = (id: string) =>
    db.db.prepare("SELECT requester,state FROM friends WHERE a=? OR b=?").all(id, id);
  it("adds a friend by username without regard to case, a leading @ or spaces", async () => {
    for (const name of ["bob", "BOB", "BoB", "%40bob", "%40Bob", "%20bob%20"]) {
      db.db.exec("DELETE FROM friends");
      const r = await add(name);
      expect(r.status, name).toBe(202);
      expect(await r.json()).toEqual({ state: "pending" });
      expect(pending("bob")).toEqual([{ requester: "alice", state: "pending" }]);
    }
    expect((await call("/v1/friends/alice/accept", "bob", "POST")).status).toBe(200);
    const list = await (await call("/v1/friends", "bob")).json();
    expect(list).toEqual([
      expect.objectContaining({ id: "alice", handle: "alice", state: "accepted" }),
    ]);
  });
  it("answers unknown, blocked, self and malformed names the same way", async () => {
    expect((await call("/v1/blocks/mallory", "alice", "PUT")).status).toBe(200);
    const answers = [];
    for (const name of ["nobody", "alice", "ALICE", "mallory", "a", "x".repeat(31), "a-b", "%ZZ", "%00"]) {
      const r = await add(name);
      answers.push([r.status, await r.json()]);
    }
    expect((await call("/v1/blocks/alice", "bob", "PUT")).status).toBe(200);
    const blockedByTarget = await add("bob");
    answers.push([blockedByTarget.status, await blockedByTarget.json()]);
    for (const a of answers) expect(a).toEqual([404, { error: "account_unavailable" }]);
    expect(db.db.prepare("SELECT count(*) AS n FROM friends").get()?.n).toBe(0);
  });
  it("needs a session, only accepts PUT and is rate limited", async () => {
    expect((await call("/v1/friends/by-username/bob", null, "PUT")).status).toBe(401);
    expect((await add("bob", "alice", "GET")).status).toBe(404);
    let last = 0;
    for (let i = 0; i < 21; i++) last = (await add("nobody")).status;
    expect(last).toBe(429);
  });
  it("offers no unauthenticated way to list or search usernames", async () => {
    expect((await call("/v1/friends/by-username/", "alice", "GET")).status).toBe(404);
    expect((await call("/v1/users?q=b", "alice")).status).toBe(404);
    expect((await call("/v1/friends/by-username", "alice", "PUT")).status).toBe(404);
  });
  it("keeps the account id route working for older apps", async () => {
    expect((await call("/v1/friends/bob", "alice", "PUT")).status).toBe(202);
    expect((await call("/v1/friends/alice", "alice", "PUT")).status).toBe(400);
    expect((await call("/v1/friends/nobody", "alice", "PUT")).status).toBe(404);
    expect((await call("/v1/friends/alice/accept", "bob", "POST")).status).toBe(200);
    expect((await call("/v1/friends/alice", "bob", "DELETE")).status).toBe(200);
  });

  describe("sign-up", () => {
    const pass = "correct horse battery staple";
    const submit = (email: string, name: string) =>
      handle(
        new Request(base + "/auth/register", {
          method: "POST",
          headers: {
            origin: base,
            "content-type": "application/x-www-form-urlencoded",
            "cf-connecting-ip": "ip-" + email,
          },
          body: new URLSearchParams({ email, handle: name, password: pass }),
        }),
        env,
        async () => Response.json({ id: "mail" }),
      );
    beforeEach(() => {
      env.RESEND_API_KEY = "test-mail-key";
      env.EMAIL_FROM = "Secpar <accounts@secpar.win>";
    });
    it("refuses a taken username in any letter case with 409", async () => {
      expect((await submit("one@example.com", "Casey_K")).status).toBe(200);
      for (const name of ["Casey_K", "casey_k", "CASEY_K", "cAsEy_K"]) {
        const r = await submit("two@example.com", name);
        expect(r.status, name).toBe(409);
        const page = await r.text();
        expect(page).toContain('data-error="username_taken"');
        expect(page).toContain("That username is taken");
      }
      expect(db.db.prepare("SELECT count(*) AS n FROM password_accounts").get()?.n).toBe(1);
      expect(
        db.db.prepare("SELECT handle,handle_lower FROM accounts WHERE github_id LIKE 'password:%'").all(),
      ).toEqual([{ handle: "Casey_K", handle_lower: "casey_k" }]);
      // Existing accounts count too.
      expect((await submit("three@example.com", "ALICE")).status).toBe(409);
    });
    it("gives a taken username the same answer for a known and an unknown email", async () => {
      expect((await submit("one@example.com", "Casey_K")).status).toBe(200);
      const known = await submit("one@example.com", "casey_k");
      const unknown = await submit("new@example.com", "casey_k");
      expect(known.status).toBe(409);
      expect(await known.text()).toBe(await unknown.text());
    });
  });

  describe("GitHub sign-in", () => {
    async function login(id: number, name: string) {
      const begin = await handle(new Request(base + "/auth/github"), env);
      const state = new URL(begin.headers.get("location")!).searchParams.get("state")!;
      const provider: typeof fetch = async (input) =>
        String(input).includes("access_token")
          ? Response.json({ access_token: "t" })
          : Response.json({ id, login: name, name: "Name " + name });
      const result = await handle(
        new Request(base + `/auth/callback?code=c&state=${state}`, {
          headers: { cookie: `__Host-secpar-oauth=${state}` },
        }),
        env,
        provider,
      );
      expect(result.status).toBe(302);
      const session = result.headers.getSetCookie().find((c) => c.startsWith("__Host-secpar-session="))!;
      const me = await handle(
        new Request(base + "/v1/me", { headers: { cookie: session.split(";")[0] } }),
        env,
      );
      return (await me.json()) as { id: string; handle: string };
    }
    it("turns hyphens into underscores and finds the account by either spelling", async () => {
      const me = await login(1, "Octo-Cat");
      expect(me.handle).toBe("Octo_Cat");
      expect((await add("octo_cat")).status).toBe(202);
      expect(pending(me.id)).toHaveLength(1);
      expect((await add("OCTO_CAT")).status).toBe(202);
    });
    it("picks a free suffix when the name is taken, in any case, and pads short names", async () => {
      expect((await login(1, "bob-x")).handle).toBe("bob_x");
      expect((await login(2, "BOB-X")).handle).toBe("BOB_X_2");
      expect((await login(3, "bob_x")).handle).toBe("bob_x_3");
      expect((await login(4, "ab")).handle).toBe("ab_");
      const long = "a".repeat(30);
      expect((await login(5, long)).handle).toBe(long);
      expect((await login(6, long)).handle).toBe("a".repeat(28) + "_2");
    });
    it("keeps the first username when the GitHub login changes", async () => {
      const first = await login(7, "first-name");
      const again = await login(7, "renamed");
      expect(again.id).toBe(first.id);
      expect(again.handle).toBe("first_name");
      expect(db.db.prepare("SELECT name FROM accounts WHERE id=?").get(first.id)?.name).toBe("Name renamed");
      expect((await add("renamed")).status).toBe(404);
    });
  });

  describe("migration 0005", () => {
    it("lowercases, fixes the charset and renames later duplicates", () => {
      const old = new DatabaseSync(":memory:");
      for (const f of ["0001_accounts", "0002_passwords", "0003_hosts", "0004_owner_pairings"])
        old.exec(readFileSync(new URL(`../migrations/${f}.sql`, import.meta.url), "utf8"));
      const long = "a".repeat(30);
      const seed = [
        ["id-1", "Alice"],
        ["id-2", "alice"],
        ["id-3", "ALICE"],
        ["id-4", "alice_2"],
        ["id-5", "octo-cat"],
        ["id-6", "Octo_Cat"],
        ["id-7", "x"],
        ["id-8", long],
        ["id-9", long.toUpperCase()],
        ["id-10", "z".repeat(80)],
        ["id-11", "unique-Name"],
      ];
      for (const [id, h] of seed)
        old.prepare("INSERT INTO accounts(id,github_id,handle,name) VALUES(?,?,?,?)").run(id, id, h, h);
      old.exec("INSERT INTO friends(a,b,requester,state) VALUES('id-1','id-5','id-1','pending')");
      old.exec(readFileSync(new URL("../migrations/0005_usernames.sql", import.meta.url), "utf8"));
      const rows = old.prepare("SELECT id,handle,handle_lower FROM accounts ORDER BY rowid").all() as {
        id: string; handle: string; handle_lower: string;
      }[];
      const byId = Object.fromEntries(rows.map((r) => [r.id, r]));
      expect(byId["id-1"]).toMatchObject({ handle: "Alice", handle_lower: "alice" });
      expect(byId["id-3"]).toMatchObject({ handle: "alice_3", handle_lower: "alice_3" });
      // alice_2 already exists, so the second Alice gets an id based name.
      expect(byId["id-2"].handle_lower).toBe("alice_id2");
      expect(byId["id-4"]).toMatchObject({ handle: "alice_2", handle_lower: "alice_2" });
      expect(byId["id-5"]).toMatchObject({ handle: "octo_cat", handle_lower: "octo_cat" });
      expect(byId["id-6"]).toMatchObject({ handle_lower: "octo_cat_2" });
      expect(byId["id-7"]).toMatchObject({ handle: "x__", handle_lower: "x__" });
      expect(byId["id-8"].handle_lower).toBe(long);
      expect(byId["id-9"].handle_lower).toBe("a".repeat(28) + "_2");
      expect(byId["id-10"].handle).toBe("z".repeat(30));
      expect(byId["id-11"]).toMatchObject({ handle: "unique_Name", handle_lower: "unique_name" });
      const lowers = rows.map((r) => r.handle_lower);
      expect(new Set(lowers).size).toBe(rows.length);
      for (const r of rows) expect(r.handle_lower).toMatch(/^[a-z0-9_]{3,30}$/);
      // Friendships stay and the scratch table is gone.
      expect(old.prepare("SELECT count(*) AS n FROM friends").get()?.n).toBe(1);
      expect(old.prepare("SELECT name FROM sqlite_master WHERE name='handle_fix'").get()).toBeUndefined();
      expect(() =>
        old.prepare("INSERT INTO accounts(id,github_id,handle,handle_lower,name) VALUES('n','n','ALICE','alice','n')").run(),
      ).toThrow();
      // A row from an older Worker without handle_lower is still accepted.
      old.prepare("INSERT INTO accounts(id,github_id,handle,name) VALUES('late','late','Late','Late')").run();
    });
  });
});

describe("user directory", () => {
  const list = async (query: string, actor: keyof typeof tokens = "alice") => {
    const r = await call("/v1/directory" + query, actor);
    expect(r.status).toBe(200);
    return (await r.json()) as { handle: string; relation: string; cursor: string }[];
  };
  const show = (actor: keyof typeof tokens) =>
    call("/v1/me/visibility", actor, "PUT", { visible: true });
  it("hides every account until its owner opts in", async () => {
    expect(await list("")).toEqual([]);
    expect(await list("?q=bob")).toEqual([]);
    expect((await show("bob")).status).toBe(200);
    expect((await list("?q=bo")).map((r) => r.handle)).toEqual(["bob"]);
    expect(((await (await call("/v1/me", "bob")).json()) as { visible: boolean }).visible).toBe(true);
    expect((await call("/v1/me/visibility", "bob", "PUT", { visible: false })).status).toBe(200);
    expect(await list("?q=bob")).toEqual([]);
  });
  it("validates the visibility value and needs a session", async () => {
    expect((await call("/v1/me/visibility", "bob", "PUT", { visible: "yes" })).status).toBe(400);
    expect((await call("/v1/me/visibility", null, "PUT", { visible: true })).status).toBe(401);
    expect((await call("/v1/directory", null)).status).toBe(401);
  });
  it("searches by prefix without regard to case and treats wildcards as plain text", async () => {
    await show("bob");
    await show("mallory");
    expect((await list("?q=B")).map((r) => r.handle)).toEqual(["bob"]);
    expect((await list("?q=%40MAL")).map((r) => r.handle)).toEqual(["mallory"]);
    expect((await list("")).map((r) => r.handle)).toEqual(["bob", "mallory"]);
    for (const bad of ["%25", "_", "b_b", "b%25", "a-b", "x".repeat(31)]) {
      expect(await list("?q=" + bad), bad).toEqual([]);
    }
  });
  it("leaves out the caller and anyone blocked in either direction", async () => {
    await show("alice");
    await show("bob");
    await show("mallory");
    expect((await list("")).map((r) => r.handle)).toEqual(["bob", "mallory"]);
    await call("/v1/blocks/bob", "alice", "PUT");
    expect((await list("")).map((r) => r.handle)).toEqual(["mallory"]);
    await call("/v1/blocks/alice", "mallory", "PUT");
    expect(await list("")).toEqual([]);
  });
  it("shows the relation and pages by cursor", async () => {
    await show("bob");
    await show("mallory");
    expect((await list("")).map((r) => r.relation)).toEqual(["none", "none"]);
    await call("/v1/friends/by-username/bob", "alice", "PUT");
    expect((await list("")).map((r) => r.relation)).toEqual(["requested", "none"]);
    expect((await list("", "bob")).map((r) => r.relation)).toEqual(["none"]);
    await show("alice");
    expect((await list("", "bob")).map((r) => r.relation)).toEqual(["incoming", "none"]);
    await call("/v1/friends/alice/accept", "bob", "POST");
    expect((await list("")).map((r) => r.relation)).toEqual(["friend", "none"]);
    const first = await list("");
    expect((await list("?after=" + first[0].cursor)).map((r) => r.handle)).toEqual(["mallory"]);
  });
  it("returns at most 20 names and is rate limited", async () => {
    for (let i = 0; i < 25; i++) {
      const name = "user_" + String(i).padStart(2, "0");
      await db
        .prepare("INSERT INTO accounts(id,github_id,handle,handle_lower,name,visible) VALUES(?,?,?,?,?,1)")
        .bind("u" + i, "u" + i, name, name, "u")
        .run();
    }
    expect(await list("?q=user_")).toHaveLength(20);
    expect(await list("?q=userx")).toEqual([]);
    let last = 0;
    for (let i = 0; i < 60; i++) last = (await call("/v1/directory?q=a")).status;
    expect(last).toBe(429);
  });
  it("never lists invisible accounts and leaks no ids or emails", async () => {
    await show("bob");
    const rows = await list("");
    expect(JSON.stringify(rows)).not.toContain("mallory");
    expect(Object.keys(rows[0]).sort()).toEqual(["cursor", "handle", "name", "relation"]);
  });
});

describe("per-friend host settings", () => {
  const settings = (actor: keyof typeof tokens, friend: string, data?: unknown) =>
    call(`/v1/friends/${friend}/settings`, actor, data ? "PUT" : "GET", data);
  const ask = (host: string, permissions: string[], actor: keyof typeof tokens = "bob") =>
    call("/v1/requests", actor, "POST", { computer: host, permissions });
  it("defaults to view only and ask each time", async () => {
    await friendship();
    expect(await (await settings("alice", "bob")).json()).toEqual({ input: "view", connect: "ask", keyboard: false, mouse: false });
    const list = await (await call("/v1/friends", "alice")).json();
    expect(list[0]).toMatchObject({ input: "view", connect: "ask" });
  });
  it("is only for the owner and only for accepted friends", async () => {
    expect((await settings("alice", "bob", { input: "full" })).status).toBe(404);
    await friendship();
    expect((await settings("alice", "mallory", { input: "full" })).status).toBe(404);
    expect((await settings("alice", "bob", { input: "full" })).status).toBe(200);
    expect(await (await settings("bob", "alice")).json()).toEqual({ input: "view", connect: "ask", keyboard: false, mouse: false });
    expect((await settings("alice", "bob", {})).status).toBe(400);
    expect((await settings("alice", "bob", { input: "admin" })).status).toBe(400);
    expect((await settings("alice", "bob", { connect: "never" })).status).toBe(400);
    expect((await settings("alice", "bob", { connect: "always" })).status).toBe(200);
    expect(await (await settings("alice", "bob")).json()).toEqual({ input: "full", connect: "always", keyboard: true, mouse: true });
  });
  it("strips keyboard and mouse for a view only friend and refuses input only requests", async () => {
    await friendship();
    const host = await computer();
    const r = await ask(host, ["view", "keyboard", "mouse"]);
    expect(r.status).toBe(201);
    const made = await r.json();
    expect(made).toMatchObject({ state: "pending", permissions: ["view"] });
    expect((await ask(host, ["keyboard"])).status).toBe(403);
    expect(
      (await call(`/v1/requests/${made.id}/approve`, "alice", "POST", { permissions: ["view", "mouse"] })).status,
    ).toBe(400);
    expect(
      (await call(`/v1/requests/${made.id}/approve`, "alice", "POST", { permissions: ["view"] })).status,
    ).toBe(200);
  });
  it("lets keyboard and mouse be allowed apart and tells the host", async () => {
    await friendship();
    const host = await computer();
    expect((await settings("alice", "bob", { keyboard: true })).status).toBe(200);
    expect(await (await settings("alice", "bob")).json()).toEqual({ input: "view", connect: "ask", keyboard: true, mouse: false });
    const made = await (await ask(host, ["view", "keyboard", "mouse"])).json();
    expect(made.permissions).toEqual(["keyboard", "view"]);
    expect((await call(`/v1/requests/${made.id}/approve`, "alice", "POST", { permissions: ["view", "mouse"] })).status).toBe(400);
    expect((await settings("alice", "bob", { mouse: true })).status).toBe(200);
    expect(await (await settings("alice", "bob")).json()).toEqual({ input: "full", connect: "ask", keyboard: true, mouse: true });
    expect((await settings("alice", "bob", { keyboard: "yes" })).status).toBe(400);
    // Choosing the single setting again decides both.
    await settings("alice", "bob", { input: "view" });
    expect(await (await settings("alice", "bob")).json()).toEqual({ input: "view", connect: "ask", keyboard: false, mouse: false });
    const list = await (await call("/v1/friends", "alice")).json();
    expect(list[0]).toMatchObject({ keyboard: false, mouse: false });
  });
  it("keeps input in the grant only for a friend with full access and revokes it when lowered", async () => {
    await friendship();
    const host = await computer();
    await settings("alice", "bob", { input: "full" });
    const made = await (await ask(host, ["view", "keyboard", "mouse"])).json();
    expect(made.permissions).toEqual(["keyboard", "mouse", "view"]);
    expect(
      (await call(`/v1/requests/${made.id}/approve`, "alice", "POST", { permissions: ["view", "keyboard"] })).status,
    ).toBe(200);
    await settings("alice", "bob", { input: "view" });
    const row = await (await call(`/v1/requests/${made.id}`, "alice")).json();
    expect(row.state).toBe("revoked");
  });
  it("approves at once when the friend may connect at any time", async () => {
    await friendship();
    const host = await computer();
    await settings("alice", "bob", { connect: "always" });
    const made = await (await ask(host, ["view", "audio"])).json();
    expect(made).toMatchObject({ state: "approved", permissions: ["audio", "view"] });
    expect((await call(`/v1/requests/${made.id}/claim`, "bob", "POST")).status).toBe(200);
  });
  it("refuses a blocked friend even with connect at any time", async () => {
    await friendship();
    const host = await computer();
    await settings("alice", "bob", { connect: "always" });
    await call("/v1/blocks/bob", "alice", "PUT");
    expect((await ask(host, ["view"])).status).toBe(403);
  });
  it("names the guest and the computer on connection requests", async () => {
    await friendship();
    const host = await computer();
    await ask(host, ["view"]);
    const rows = await (await call("/v1/requests", "alice")).json();
    expect(rows[0]).toMatchObject({ guest_handle: "bob", computer_name: "Desktop", state: "pending" });
  });
  it("gives a host its friends only with the machine credential", async () => {
    expect((await call("/v1/host/friends", "alice")).status).toBe(401);
  });
  it("lets the host change what a friend may do and sees it at once", async () => {
    await friendship();
    const host = await enrolledHost();
    const bob = (await (await machine("/v1/host/friends", host.credential)).json()).find((f: { handle: string }) => f.handle === "bob");
    expect(bob).toMatchObject({ input: "view", connect: "ask", keyboard: false, mouse: false });
    expect((await machine(`/v1/host/friends/${bob.id}/settings`, host.credential, "PUT", { keyboard: true, connect: "always" })).status).toBe(200);
    const after = (await (await machine("/v1/host/friends", host.credential)).json()).find((f: { handle: string }) => f.handle === "bob");
    expect(after).toMatchObject({ connect: "always", keyboard: true, mouse: false });
    expect((await machine(`/v1/host/friends/${bob.id}/settings`, host.credential, "PUT", { connect: "never" })).status).toBe(400);
    expect((await machine(`/v1/host/friends/${"0".repeat(8)}-0000-4000-8000-000000000000/settings`, host.credential, "PUT", { mouse: true })).status).toBe(404);
    // The signed-in owner sees the same row.
    expect(await (await call(`/v1/friends/${bob.id}/settings`, "alice")).json()).toMatchObject({ connect: "always", keyboard: true });
  });
});

describe("host presence for friends", () => {
  const PC = "00000000-0000-4000-8000-0000000000c1";
  it("lists a host only while it says it is sharing and tells the claiming friend where it is", async () => {
    await friendship();
    const host = await enrolledHost();
    const beat = (data?: unknown) => machine("/v1/host/heartbeat", host.credential, "POST", data);
    expect(await (await beat()).json()).toMatchObject({ sharing: false });
    expect(await (await call("/v1/friend-computers", "bob")).json()).toEqual([]);
    expect((await (await beat({ sharing: true, pc: PC, relay: "https://relay.example" })).json()).sharing).toBe(true);
    expect((await (await call("/v1/friend-computers", "bob")).json()).map((c: { id: string }) => c.id)).toEqual([host.computer]);
    const made = await (await call("/v1/requests", "bob", "POST", { computer: host.computer, permissions: ["view"] })).json();
    expect((await call(`/v1/requests/${made.id}/approve`, "alice", "POST", { permissions: ["view"] })).status).toBe(200);
    const claimed = await (await call(`/v1/requests/${made.id}/claim`, "bob", "POST")).json();
    expect(claimed.host).toEqual({ pc: PC, relay: "https://relay.example", fingerprint: host.certificate_fp });
  });
  it("refuses a claim when the host stopped sharing and keeps the grant for later", async () => {
    await friendship();
    const host = await enrolledHost();
    const beat = (data?: unknown) => machine("/v1/host/heartbeat", host.credential, "POST", data);
    await beat({ sharing: true, pc: PC, relay: "https://relay.example" });
    const made = await (await call("/v1/requests", "bob", "POST", { computer: host.computer, permissions: ["view"] })).json();
    await call(`/v1/requests/${made.id}/approve`, "alice", "POST", { permissions: ["view"] });
    await beat({ sharing: false });
    expect(await (await call("/v1/friend-computers", "bob")).json()).toEqual([]);
    const refused = await call(`/v1/requests/${made.id}/claim`, "bob", "POST");
    expect([refused.status, (await refused.json()).error]).toEqual([409, "host_not_sharing"]);
    await beat({ sharing: true, pc: PC, relay: "https://relay.example" });
    expect((await call(`/v1/requests/${made.id}/claim`, "bob", "POST")).status).toBe(200);
  });
  it("rejects presence that is not a plain https origin or a lowercase id", async () => {
    const host = await enrolledHost();
    const beat = (data: unknown) => machine("/v1/host/heartbeat", host.credential, "POST", data);
    for (const bad of [
      { sharing: true, pc: PC, relay: "http://relay.example" },
      { sharing: true, pc: PC, relay: "https://relay.example/path" },
      { sharing: true, pc: PC, relay: "https://user:pass@relay.example" },
      { sharing: true, pc: PC, relay: "https://relay.example/?x=1" },
      { sharing: true, pc: "not-an-id", relay: "https://relay.example" },
      { sharing: true, relay: "https://relay.example" },
    ]) {
      expect((await beat(bad)).status).toBe(400);
    }
    expect(db.db.prepare("SELECT sharing FROM host_keys").get()!.sharing).toBe(0);
  });
});
