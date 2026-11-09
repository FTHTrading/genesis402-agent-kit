// On-chain settlement matching for btc402 attestations (Base mainnet USDC). Node wrapper over the browser-safe chain-core.
import { fetchUsdcTransfer as fetchCore, DEFAULT_TIMEOUT_MS } from './chain-core.mjs';

export { USDC, TRANSFER_TOPIC, TREASURY, matchSettlement, DEFAULT_TIMEOUT_MS } from './chain-core.mjs';

export const timeoutMs = () => Number(process.env.BTC402_FETCH_TIMEOUT_MS) || DEFAULT_TIMEOUT_MS;

// Same contract as chain-core's fetchUsdcTransfer; the default timeout honours BTC402_FETCH_TIMEOUT_MS and fetch defaults to the global.
export function fetchUsdcTransfer(txHash, { rpc, fetchImpl = fetch, timeoutMs: t = timeoutMs() } = {}) {
  return fetchCore(txHash, { rpc, fetchImpl, timeoutMs: t });
}
