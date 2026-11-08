# Secpar account and friend service (development)

This is a separate Cloudflare Worker with D1 storage. It does not change the original PC Remote relay, existing owner pairings or Preview 10. The API is deployed at https://accounts.secpar.win. Desktop sign-in/friend UI is implemented in development source; host/Android integration is not complete. Email/password is the default sign-in. The Resend sender is configured, registration/reset pages are enabled, and Resend accepted a simulated-delivery probe from the configured sender. The user confirmed registration, email verification and browser sign-in on 2026-11-05.

Implemented HTTP endpoints provide email/password registration, email verification, sign-in and password reset, optional GitHub OAuth with browser-bound one-use state and PKCE, hashed opaque account sessions, logout/revocation, native device approval/polling, friend requests by username with explicit acceptance, blocking/unblocking, owner computer metadata, guest connection requests, owner approval of a subset of requested permissions, one-use guest claims and short-lived ES256 admission grants. No endpoint shares an existing owner device token, PIN or relay master key.

Guest grants are restricted to view/audio/gamepad/keyboard/mouse/clipboard/files. Unlock, power and other administrator actions cannot be requested. The host must verify issuer, audience, signature, expiry, intended guest and grant id, then check the grant's active status on this service before accepting it. It must keep checking the lease and disconnect on revocation, block, lost friendship or lease expiry. JWT signature verification alone does not provide immediate revocation. Admission tokens expire after at most 60 seconds; an accepted session lease lasts at most eight hours. Host enforcement and gamepad-seat assignment are still to be implemented; these API tests are not proof that multiplayer is usable.

## Deployment and email setup

1. The configured account-service origin is https://accounts.secpar.win, on the user's secpar.win domain.
2. Use the Edit Cloudflare Workers token template with Account D1 Edit and Zone Read added, limited to the account and secpar.win zone. Save the scoped Cloudflare API token in the Secpar repository's CLOUDFLARE_API_TOKEN Actions secret. The manual Deploy Secpar Cloudflare services workflow finds the account owning secpar.win, creates/reuses only secpar-accounts, generates a deployment configuration and applies its migrations. The checked-in all-zero database id is a placeholder, not a deployment target.
3. Create a free Resend account and verify the sending subdomain `mail.secpar.win`. Add only the DNS records Resend supplies for that subdomain; preserve existing website, relay and mailbox records. Keep open/click tracking disabled for authentication emails. Create a Sending access API key restricted to this domain and add it to the repository Actions secrets as `RESEND_API_KEY`. The deployment workflow transfers it directly to the Worker secret store without displaying its value. `EMAIL_FROM` is `Secpar <accounts@mail.secpar.win>`. Registration and reset fail closed until both sender and credential are present. Deployment validates the sender using Resend's documented `delivered@resend.dev` simulation before transferring the key. That probe contains no account information and is not proof of real inbox delivery.

   GitHub OAuth remains an optional legacy endpoint at `/auth/github`, requiring GITHUB_CLIENT_ID and GITHUB_CLIENT_SECRET with callback `https://accounts.secpar.win/auth/callback`. It is not configured in production or required by the default sign-in.
4. The deployment workflow generates a P-256 private signing JWK once if GRANT_PRIVATE_JWK is missing. It uploads it straight to Cloudflare's secret store without logging or committing it, and preserves the key on subsequent deploys. The service publishes only its public key at /.well-known/jwks.json. Do not commit private keys or .dev.vars.
5. Run the manual deployment workflow with `service=accounts`. It reuses the database, applies outstanding migrations, preserves the signing key and checks HTTPS health. A request-only public DNS fallback handles stale runner DNS caches without changing system settings. Local fixtures must not be used as production sessions or signing keys.

The hourly maintenance trigger removes expired email/OAuth/device tokens, sessions, rate-limit buckets and old connection requests. Device and OAuth endpoints have IP-based request limits; provider/API calls have timeouts. Public account signup still needs production abuse monitoring and a deployment review.

## Native sign-in

POST /auth/device with a device name. Show the user_code and open verification_uri_complete in the user's browser. The user signs in and explicitly approves the matching code. Poll POST /auth/device/token using the separate device_code no more frequently than the returned interval. The browser never receives that polling credential. A successful poll returns a bearer token once; store it in an OS credential store. Browser mutations require an exact same-origin Origin header; native requests use bearer authorization.

## Tests

```sh
npm ci
npm run check
npm test
```

Tests use a real in-memory SQLite database with the production schema, simulated GitHub responses and actual ES256 signatures. They cover OAuth state/PKCE, hashed sessions, CSRF origin checks, native code approval and one-use exchange, friend consent, owner/guest isolation, restricted permissions, competing one-use claims, wrong-host signatures, blocking and expiry. The deployed API has passed public HTTPS/health/signing-key smoke checks. Email/password routes and the mail-disabled registration guard have also passed live HTTPS smoke checks. The configured sender has also passed Resend's simulated-delivery check. The user confirmed real inbox delivery, verification and browser sign-in. Native approval/friend actions still need real-user validation, and host enrollment proofs, host grant enforcement and host/Android UI integration remain outstanding.

## Password sign-in

`GET /auth/start` opens `/auth/sign-in`, preserving the native activation code. The browser flow uses ordinary forms without JavaScript or biometrics. Routes `/auth/register`, `/auth/forgot`, `/auth/verify` and `/auth/reset` handle registration and recovery. All POSTs require the exact account-service Origin. Cookies are Secure, HttpOnly and SameSite=Lax; tokens and sessions are hashed in D1. Verification/reset links expire after 30 minutes, are consumed only by an explicit POST and cannot be replayed. Resetting a password invalidates every existing account session and other recovery links.

Passwords accept 8–256 characters with whitespace preserved and no arbitrary composition rules. Native `node:crypto` scrypt uses a random salt and N=16384, r=8, p=5 (the [OWASP 16 MiB configuration](https://cheatsheetseries.owasp.org/cheatsheets/Password_Storage_Cheat_Sheet.html#scrypt)). Missing-account logins perform the same KDF; responses do not identify whether an email exists. Login/signup/reset have separate per-IP and per-email limits. No plaintext password is persisted or sent to the email provider. Password hashing was also exercised in local workerd, beyond Node tests; a deployed missing-account login also completed the same password KDF and returned the expected 401. Concurrent-load/resource-limit testing remains outstanding. The [Cloudflare Node crypto API](https://developers.cloudflare.com/workers/runtime-apis/nodejs/crypto/) requires nodejs_compat.

An email/password account gets an internal `password:<uuid>` identity in the legacy non-null github_id column. It is never linked to a GitHub identity merely because email addresses match. Existing GitHub account rows and owner pairings are preserved. Unverified accounts cannot sign in; an email owner can recover an existing unverified signup by resetting its password. Existing registrations do not overwrite stored credentials.

Browser form pages use `Referrer-Policy: same-origin`. Using `no-referrer` makes browsers send `Origin: null` on these POSTs, which conflicts with the exact-origin CSRF check. The origin check remains enforced, and no referrer is sent to other sites. The failure was reproduced with a real browser form before the fix.

## Usernames and friends

A username is `accounts.handle`. It has 3 to 30 characters from `A-Z a-z 0-9 _` and is unique without regard to case. Migration `0005_usernames.sql` adds `handle_lower` with a unique index. The first sign-up to use a name keeps it. The migration renames later duplicates to `name_2`, `name_3` and so on, and maps hyphens to underscores. `handle_lower` stays nullable so a Worker from before the migration can still insert rows during a deploy.

Password sign-up stores the name as typed and answers `409` with `data-error="username_taken"` on the page when any account has the name in any letter case. The answer depends only on the username, so it says nothing about email addresses. GitHub sign-in maps every character outside letters and digits to an underscore, pads names under three characters with underscores, and adds `_2`, `_3` and so on when the name is taken. The username is chosen once. Later logins update the display name and keep the username.

`PUT /v1/friends/by-username/:name` sends a friend request. The name is trimmed, loses one leading `@` and is lowercased before lookup. Unknown names, malformed names, your own name and blocked accounts all answer `404 {"error":"account_unavailable"}`. A match answers `202 {"state":"pending"}` and uses the same pending-invite insert as the id route. The route shares the `friend` rate limit (20 per minute per address) and counts every attempt, so it cannot be used to scan names. Names are listed only through the opt-in directory in `docs/friends.md` (migration `0006_directory_and_friend_settings.sql`). `GET /v1/me` returns the caller's own `handle`.

`PUT /v1/friends/:id` still works with an account id for older app versions. Accept, remove and block use ids.

## Page styling

The browser pages share `src/ui.ts` (layout, stylesheet, wordmark). `src/assets.ts` holds the Rubik 400 and 500 files and the wordmark as text, copied from `cloud/site/public/assets`. The Worker serves them under `/assets/` so the content security policy only allows `self`.
