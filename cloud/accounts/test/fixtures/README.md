These certificate/private-key files are a disposable, publicly committed P-256 test
identity generated specifically for enrollment tests. They contain no user or
production keys. Never configure an actual Secpar host with this identity.

Service tests use only the in-memory database and accounts.example.test. The Rust
signature interoperability vector uses the production audience string to check the
protocol, but has a fixed fixture owner/nonce/time and is never sent to a live API.

`guest-grant.json` is another public fixture signed with the same disposable key.
It exercises the account service's ES256 grant format in the Rust verifier and
uses a fixed fixture PC, guest, request and time. It is never a live admission token.
