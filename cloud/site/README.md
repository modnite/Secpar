# Secpar website

The public site uses `secpar.win`, with `www.secpar.win` redirecting to it. The
account service stays on `accounts.secpar.win`. Static assets are deployed through
the Secpar repository's existing Cloudflare account, without a separate backend
database or analytics service.

Run `node cloud/site/generate-downloads.mjs` to resolve current, verified GitHub
release assets. Preview the public directory with a local static server. The
deployment workflow refreshes downloads when a release is published.

The visual direction takes typography, hierarchy and spacing inspiration from
Parsec's public site. Rubik is self-hosted under the included OFL license. Secpar
uses its approved mint/orange triangle and wordmark. Interface imagery is a
clearly labelled sample; no customer, latency or unsupported-platform claims are
invented. Linux hosting limitations and the unfinished macOS product remain
visible. There are no trackers, external font requests or cookie dialogs.
