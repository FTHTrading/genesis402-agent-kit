# Security

Report a vulnerability privately through GitHub private vulnerability reporting for this repository:
https://github.com/FTHTrading/genesis402-agent-kit/security/advisories/new

Please do not open a public issue for security reports. Reports are acknowledged within 3 business days.

## Scope
- `genesis402-mcp` (this package) and the hosted endpoint `https://twin.unykorn.org/mcp`.
- The rail's 360 endpoints are in scope for authorization and payment-bypass issues: a paid call returning data without a settled payment, receipt forgery, replay of a payment.

## What this client guarantees
- Quote-only by default: nothing is signed or paid unless `GENESIS402_LIVE=1` and `GENESIS402_PAYER_KEY` are set; any quote above `GENESIS402_MAX_USD` (default $0.25) is refused, both against the quote and again by the x402 client when it signs; an invalid cap keeps the server quote-only.
- The hosted endpoint holds no keys; payments are signed by the caller's own wallet.
- Releases are built on GitHub Actions and published with SLSA provenance through npm's trusted publisher; no long-lived npm token exists.

## Supported versions
Only the latest published version receives fixes.
