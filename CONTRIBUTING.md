# Contributing

This repository holds the Genesis402 MCP server (`mcp/`) and quickstart payers.

- **Run the tests before a PR:** `cd mcp && npm ci && npm test` (read-only against the live rail; it never pays).
- **Never commit a key.** `GENESIS402_PAYER_KEY` belongs in your environment only. CI runs with `GENESIS402_LIVE=0`.
- **Prices come from the rail.** Do not hard-code a price anywhere in this client; tool descriptions read the live manifest.
- **Releases** are cut by tagging: bump `mcp/package.json`, `mcp/server.json` (both version fields) and `VERSION` in `mcp/core.mjs` to the same number, add a `CHANGELOG.md` entry, then `git tag -a vX.Y.Z && git push origin vX.Y.Z`. The `release-mcp` workflow publishes to npm (OIDC, provenance) and the MCP Registry.
- **Wording on public surfaces** follows the repository marketing-copy ADR: no claims of custody, insurance, guarantees or regulatory status.
