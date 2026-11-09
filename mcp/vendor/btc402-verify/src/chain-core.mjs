// Browser-safe core of the Base USDC settlement check (no Node imports, no process.env). Public RPC reads only.
// src/chain.mjs re-exports this for Node; the proof page loads a copy of this file as /proof/chain-core.mjs.
export const USDC = '0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913';
export const TRANSFER_TOPIC = '0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef';
// The published UnyKorn treasury (ruling R17). A settlement only counts when it pays this address, whatever the record says.
export const TREASURY = '0x7d9a65d06dcc435a52D5880C6310Bd6E96c156DB';
export const NOT_TREASURY = 'pay_to is not the UnyKorn treasury';

const PASS = 'PASS', FAIL = 'FAIL';
const check = (name, result, detail) => ({ name, result, detail });
const topicAddr = (t) => '0x' + String(t).slice(-40).toLowerCase();

export const DEFAULT_TIMEOUT_MS = 15000;

// Returns {status: number, block: number, from: string|null, to: string|null, amount: bigint|null} for the first USDC
// Transfer log of the tx, or null if there is no receipt. from/to/amount are null when the receipt has no USDC Transfer log.
// Throws on network, timeout (opts.timeoutMs, default 15 s), HTTP or JSON-RPC errors.
export async function fetchUsdcTransfer(txHash, { rpc, fetchImpl = (...a) => globalThis.fetch(...a), timeoutMs: t = DEFAULT_TIMEOUT_MS }) {
  const res = await fetchImpl(rpc, {
    signal: AbortSignal.timeout(t),
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'eth_getTransactionReceipt', params: [txHash] }),
  });
  if (!res.ok) throw new Error(`RPC HTTP ${res.status}`);
  const body = await res.json();
  if (body.error) throw new Error(`RPC error: ${body.error.message ?? JSON.stringify(body.error)}`);
  const rc = body.result;
  if (!rc) return null;
  const log = (rc.logs ?? []).find((l) => l.address?.toLowerCase() === USDC.toLowerCase() && l.topics?.[0] === TRANSFER_TOPIC);
  return {
    status: Number(BigInt(rc.status)),
    block: Number(BigInt(rc.blockNumber)),
    from: log ? topicAddr(log.topics[1]) : null,
    to: log ? topicAddr(log.topics[2]) : null,
    amount: log ? BigInt(log.data) : null,
  };
}

const lc = (s) => String(s ?? '').toLowerCase();

// Compare the attestation's claims with the on-chain transfer.
export function matchSettlement(att, transfer) {
  if (!transfer) {
    return ['tx_success', 'payer', 'pay_to', 'amount'].map((n) => check(n, FAIL, 'no receipt'));
  }
  const noLog = transfer.from == null;
  const addr = (name, want, got, label) => (noLog
    ? check(name, FAIL, 'no USDC Transfer log in receipt')
    : check(name, lc(want) === lc(got) ? PASS : FAIL,
      lc(want) === lc(got) ? `${label} ${got}` : `record says ${want}, chain says ${got}`));
  let want = null;
  try { want = BigInt(att?.execution?.amount_paid); } catch { /* malformed amount stays null */ }
  const amount = noLog ? check('amount', FAIL, 'no USDC Transfer log in receipt')
    : check('amount', want !== null && want === transfer.amount ? PASS : FAIL,
      want !== null && want === transfer.amount ? `${transfer.amount} atomic USDC`
        : `record says ${att?.execution?.amount_paid}, chain says ${transfer.amount}`);
  let payTo = addr('pay_to', att?.authority?.pay_to, transfer.to, 'USDC to');
  // Pinned: record and chain agreeing on some other recipient is still a FAIL.
  if (payTo.result === PASS && lc(transfer.to) !== lc(TREASURY)) payTo = check('pay_to', FAIL, NOT_TREASURY);
  return [
    check('tx_success', transfer.status === 1 ? PASS : FAIL,
      transfer.status === 1 ? `status 1 in block ${transfer.block}` : `status ${transfer.status} in block ${transfer.block}`),
    addr('payer', att?.subject?.agent_wallet, transfer.from, 'USDC from'),
    payTo,
    amount,
  ];
}
