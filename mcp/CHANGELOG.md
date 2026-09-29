# Changelog

All notable changes to `genesis402-mcp`. Dates are UTC. Releases are built and signed on GitHub Actions (SLSA provenance) and published to npm and the MCP Registry by `.github/workflows/release-mcp.yml`.

## 0.3.5 - 2026-09-29
- Type declarations (`index.d.ts`) for `createServer`, `localPayerFromEnv`, `loadCatalog`, `catalogSize`, `VERSION`.
- `exports` map: `import { createServer } from "genesis402-mcp"` resolves to the library (`core.mjs`); the CLI entry is unchanged (`npx genesis402-mcp`).
- Package metadata: homepage, bugs, `sideEffects`; this changelog ships in the tarball. README carries the banner and badges on npm.

## 0.3.4 - 2026-09-28
- Every tool declares an `outputSchema` and returns `structuredContent` alongside text.

## 0.3.3 - 2026-09-28
- Tool descriptions are built from the rail's live `/.well-known/x402` manifest at startup, so prices never go stale in the client.
- Server `instructions` in the handshake; environment defaults documented.

## 0.3.2 - 2026-09-26
- Every tool and parameter described; UnyKorn title and icons in `server.json`. First release on the MCP Registry and Glama.

## 0.3.1 - 2026-09-25
- README rewrite with ADR-approved wording; MIT LICENSE; Dockerfile; `glama.json`.

## 0.3.0 - 2026-09-24
- Hosted streamable-HTTP endpoint (keyless) at `https://twin.unykorn.org/mcp`; real endpoint paths from the manifest; batch-3 named tools.

## 0.1.0 - 2026-09-17
- Initial MCP server: 8 tools, quote-only default, spend cap; Node and Python quickstart payers for Base USDC.
