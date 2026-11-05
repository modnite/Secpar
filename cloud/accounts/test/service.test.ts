import { beforeEach, describe, expect, it } from "vitest";
import { DatabaseSync } from "node:sqlite";
import { readFileSync } from "node:fs";
import { generateKeyPair, exportJWK, importJWK, jwtVerify } from "jose";
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
  return (await r.json()).id as string;
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
      .prepare("INSERT INTO accounts(id,github_id,handle,name) VALUES(?,?,?,?)")
      .bind(id, id, id, id)
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
      (await form("/auth/register", { ...fields, password: nextPass }, send))
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
