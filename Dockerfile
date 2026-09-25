# Genesis402 MCP server (stdio), quote-only by default.
#
# Build:  docker build -t genesis402-mcp .
# Run:    docker run --rm -i genesis402-mcp
#
# No secrets are baked into this image. Without GENESIS402_LIVE=1 and
# GENESIS402_PAYER_KEY passed at run time, every paid tool returns the x402
# quote and nothing is signed or paid.
FROM node:24.16.0-slim

ENV NODE_ENV=production

WORKDIR /app

COPY mcp/package.json mcp/package-lock.json ./
RUN npm ci --omit=dev && npm cache clean --force

COPY mcp/index.mjs mcp/core.mjs mcp/http.mjs mcp/README.md ./

USER node

CMD ["node", "index.mjs"]
