// Type declarations for genesis402-mcp. Hand-maintained; mirrors the exports of core.mjs.
import type { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";

/** Package version, also reported as serverInfo.version in the MCP handshake. */
export declare const VERSION: string;

/**
 * Load (or refresh) the endpoint catalog from the rail's /.well-known/x402 manifest.
 * Cached for 10 minutes unless `force` is true. Never throws: on failure the previous catalog is kept.
 */
export declare function loadCatalog(force?: boolean): Promise<void>;

/** Number of endpoints currently in the catalog (0 until the first successful load). */
export declare const catalogSize: () => number;

/** A payer: a fetch wrapped with x402 payment plus the per-call USD cap it enforces. */
export interface Payer {
  /** fetch that settles 402 challenges with the configured wallet. */
  paidFetch: typeof fetch;
  /** Hard cap per call in USD; quotes above it are refused, never paid. */
  maxUsd: number;
}

/**
 * Build a payer from the environment, or `null` (quote-only) unless BOTH
 * GENESIS402_LIVE=1 and GENESIS402_PAYER_KEY are set. Cap: GENESIS402_MAX_USD (default 0.25).
 */
export declare function localPayerFromEnv(): Payer | null;

export interface CreateServerOptions {
  /** Payer to settle calls with; `null` keeps every paid tool quote-only. */
  payer?: Payer | null;
  /** Hosted mode (keyless): paid tools return the x402 quote and accept a client-supplied payment_signature. */
  hosted?: boolean;
}

/** Create the Genesis402 MCP server with all tools registered. Connect it to any MCP transport. */
export declare function createServer(options?: CreateServerOptions): Promise<McpServer>;
