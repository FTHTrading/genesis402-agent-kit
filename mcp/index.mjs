#!/usr/bin/env node
// Genesis402 MCP server over stdio (npx genesis402-mcp). All tool logic lives in core.mjs.
// Quote-only unless GENESIS402_LIVE=1 and GENESIS402_PAYER_KEY are set (cap: GENESIS402_MAX_USD, default 0.25).
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { createServer, localPayerFromEnv } from "./core.mjs";

const server = await createServer({ payer: localPayerFromEnv() });
await server.connect(new StdioServerTransport());
