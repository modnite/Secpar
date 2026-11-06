# Wake on LAN

Secpar sends standard magic packets directly on the local network. No Home Assistant account or extra service is needed. The computer needs Wake on LAN enabled in its firmware/network adapter and a wired connection that stays powered while asleep.

For wake requests from outside that network, Settings → Wake on LAN offers an optional HTTPS wake link. This can point to an authenticated endpoint on a router, NAS or another always-on device that can send a magic packet on the sleeping computer’s LAN. A Cloudflare Worker alone cannot broadcast into a home network or wake a powered-off computer without such a helper.

Secpar sends an HTTPS POST with JSON `{ "macs": ["AA:BB:CC:DD:EE:FF"] }`. An optional bearer token is sent in the Authorization header. Configure the helper to authorize only approved computers/MAC addresses and send their magic packets; do not expose arbitrary shell execution. Redirects are not followed. The client encrypts both the URL (which can itself be a secret) and bearer token in its settings.

Existing Home Assistant settings continue working under the optional Home Assistant method. The local-network option can be selected at any time without deleting those settings. “Wake request sent” means the request was transmitted or accepted by the helper, not that the computer has already booted.
