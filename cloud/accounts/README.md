# Secpar account and friend service (development)

This is a separate Cloudflare Worker with D1 storage. It does not change the original PC Remote relay, existing owner pairings or Preview 10. It is not deployed, and the desktop/host account integration is not complete.

Implemented HTTP endpoints provide GitHub OAuth sign-in with browser-bound one-use state and PKCE, hashed opaque account sessions, logout/revocation, native device approval/polling, friend requests with explicit acceptance, blocking/unblocking, owner computer metadata, guest connection requests, owner approval of a subset of requested permissions, one-use guest claims and short-lived ES256 admission grants. No endpoint shares an existing owner device token, PIN or relay master key.

Guest grants are restricted to view/audio/gamepad/keyboard/mouse/clipboard/files. Unlock, power and other administrator actions cannot be requested. The host must verify issuer, audience, signature, expiry, intended guest and grant id, then check the grant's active status on this service before accepting it. It must keep checking the lease and disconnect on revocation, block, lost friendship or lease expiry. JWT signature verification alone does not provide immediate revocation. Admission tokens expire after at most 60 seconds; an accepted session lease lasts at most eight hours. Host enforcement and gamepad-seat assignment are still to be implemented; these API tests are not proof that multiplayer is usable.

## Configuration before deployment

1. Decide the real HTTPS account-service origin. The example.invalid origin deliberately refuses authentication.
2. Sign in to Cloudflare with Wrangler and create a D1 database named secpar-accounts. Replace the all-zero database id in wrangler.jsonc.
3. Create a GitHub OAuth application with callback URL `https://YOUR-ACCOUNT-ORIGIN/auth/callback`. Set GITHUB_CLIENT_ID in the Worker configuration and GITHUB_CLIENT_SECRET as a Wrangler secret. GitHub is the initial identity provider; no passwords or GitHub access tokens are retained by Secpar.
4. Generate a P-256 private signing JWK and install it as the GRANT_PRIVATE_JWK Worker secret. The service publishes only its public key at /.well-known/jwks.json. Do not commit private keys or .dev.vars.
5. Apply migrations to the intended database, validate the deployed origin/provider configuration, then deploy. Local test fixtures must not be used as production sessions or signing keys.

The hourly maintenance trigger removes expired OAuth/device codes, sessions, rate-limit buckets and old connection requests. Device and OAuth endpoints have IP-based request limits; provider/API calls have timeouts. Public account signup still needs production abuse monitoring and a deployment review.

## Native sign-in

POST /auth/device with a device name. Show the user_code and open verification_uri_complete in the user's browser. The user signs in and explicitly approves the matching code. Poll POST /auth/device/token using the separate device_code no more frequently than the returned interval. The browser never receives that polling credential. A successful poll returns a bearer token once; store it in an OS credential store. Browser mutations require an exact same-origin Origin header; native requests use bearer authorization.

## Tests

```sh
npm ci
npm run check
npm test
```

Tests use a real in-memory SQLite database with the production schema, simulated GitHub responses and actual ES256 signatures. They cover OAuth state/PKCE, hashed sessions, CSRF origin checks, native code approval and one-use exchange, friend consent, owner/guest isolation, restricted permissions, competing one-use claims, wrong-host signatures, blocking and expiry. Live GitHub/Cloudflare deployment, native credential storage, host enrollment proofs, host grant enforcement and friends UI remain outstanding.
