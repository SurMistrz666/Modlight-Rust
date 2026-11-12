# Security audits

Reports on the original Modlight code (TypeScript server and Windows client) and on this Rust rewrite. All were done by static analysis; the reviews of the Rust code also include dynamic tests on a disposable stack.

| Report | Covers | Date | Verdict |
|---|---|---|---|
| [`original-client.md`](original-client.md) | The original Windows client (Modlight-App, a ShareX fork), commit `b178a09` | 2026-10-18 | Does not exfiltrate data: no backdoor, telemetry or sending to third parties. No Critical or High findings; one Medium (sign-in loopback) and a few Low |
| [`original-server-accounts-api.md`](original-server-accounts-api.md) | The original server: accounts, authentication, the API, token scopes (commit `a59d03c`) | 2026-10-18 | Does not exfiltrate data; no backdoor, escalation or IDOR. Two Medium (anonymous large-body write before the token check, TOCTOU on the first admin) and a few Low/Informational |
| [`original-server-files-exfiltration.md`](original-server-files-exfiltration.md) | The original server: files, media, serving, outgoing connections and supply chain (commit `a59d03c`) | 2026-10-18 | Does not exfiltrate data. One confirmed High (the view limit can be bypassed); the rest Low |
| [`rust-server.md`](rust-server.md) | The Rust server (`server/`), including the dynamic tests | 2026-10-20 | No Critical or High findings, no sending of data; all 7 fixes from the audits of the original are effective |
| [`rust-app.md`](rust-app.md) | The Rust macOS app (`app/`) | 2026-10-20 | No sending of data, the rules from the audit of the client preserved. One High and three Medium found in the backup import and image decoding (W1-W3, S3), all fixed afterwards |
| [`hunt-2026-11-11/`](hunt-2026-11-11/README.md) | A bug hunt over the whole Rust code base (server, web panel, app), ten areas read file by file plus a cross-area pass | 2026-11-11/12 | About 103 bugs fixed, each with a test; the high ones were a server stall on crafted HEIC/AVIF files, an encrypted upload lost when encryption was switched off, a stack overflow on deep folder chains, a crash on very large pictures in several windows, and the editor's Rotate 90 moving annotations |
