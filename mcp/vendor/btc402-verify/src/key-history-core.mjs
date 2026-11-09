// Key history window logic, shared by the Node verifier (src/verify.mjs) and the proof page (proof/proof-core.mjs).
// Browser-safe: no imports, no Node APIs. Deployed beside proof-core.mjs and served as /proof/key-history-core.mjs.
//
// History shape (GET /attestations/keys): { keys: [{ key_id, alg: 'Ed25519', public_key_hex, valid_from: ISO, valid_to: ISO | null }] }.
// A record signed by the current published key never needs the history. A record signed by a retired key passes only when its
// public key is listed AND its signed execution.timestamp (unix seconds) lies in [valid_from, valid_to], ends inclusive.
// A retired key must have an end date: valid_to null on a key that is not the published one is a misconfigured history (UNAVAILABLE).
// One entry per key: a duplicate makes the whole history malformed. An entry whose alg is not Ed25519 is treated as not in the history.

export const PASS = 'PASS', FAIL = 'FAIL', UNAVAILABLE = 'UNAVAILABLE';
export const OUTSIDE_WINDOW = "signed outside the key's validity window";
export const MISCONFIGURED = 'key history misconfigured: retired key has no end date';

const lc = (s) => String(s ?? '').toLowerCase();
const HEX64 = /^[0-9a-fA-F]{64}$/;
const ISO_UTC = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$/;

function isoToSec(v) {
  if (typeof v !== 'string' || !ISO_UTC.test(v)) return null;
  const ms = Date.parse(v);
  return Number.isFinite(ms) ? Math.floor(ms / 1000) : null;
}

// Returns [{ key_id, hex (lowercase), fromSec, toSec | null }] (Ed25519 entries only) or null when the history is missing or malformed in any way,
// including two entries for the same key.
export function parseHistory(history) {
  if (!history || typeof history !== 'object' || Array.isArray(history) || !Array.isArray(history.keys)) return null;
  const out = [];
  const seen = new Set();
  for (const e of history.keys) {
    if (!e || typeof e !== 'object' || !HEX64.test(String(e.public_key_hex)) || typeof e.key_id !== 'string' || !e.key_id) return null;
    const fromSec = isoToSec(e.valid_from);
    const toSec = e.valid_to === null || e.valid_to === undefined ? null : isoToSec(e.valid_to);
    if (fromSec === null || (e.valid_to !== null && e.valid_to !== undefined && toSec === null)) return null;
    const hex = lc(e.public_key_hex);
    if (seen.has(hex)) return null;
    seen.add(hex);
    if (e.alg !== 'Ed25519') continue;
    out.push({ key_id: e.key_id, hex, fromSec, toSec });
  }
  return out;
}

// Decide which key a record's signature must be checked against.
//   recKeyHex  the public_key_hex the record claims
//   timestamp  rec.execution.timestamp (unix seconds, inside the signed bytes)
//   current    the key published at /attestations/key ({ public_key_hex })
//   history    the parsed JSON of /attestations/keys, or null/undefined when it could not be loaded
//   nowSec     unix seconds (kept for callers; a retired key's window always has an end)
// -> { result, detail, keyHex, reason? }
//   result   PASS | FAIL | UNAVAILABLE for the key_matches_published check
//   reason   machine-readable: 'historical-key' | 'outside-window' | 'no-history' | 'not-in-history' | 'history-misconfigured'.
//            Absent for the current key and for a missing current key.
//   keyHex   the key to verify the signature with (current or historical), or null when no trusted key could be established.
//            It is never taken from the record alone.
export function resolveSigningKey({ recKeyHex, timestamp, current, history }) {
  if (!current?.public_key_hex) return { result: UNAVAILABLE, detail: 'public key unavailable', keyHex: null };
  if (lc(recKeyHex) === lc(current.public_key_hex)) {
    return { result: PASS, detail: 'signing key equals the published key', keyHex: current.public_key_hex };
  }
  const keys = parseHistory(history);
  if (!keys) return { result: UNAVAILABLE, detail: 'key history unavailable', keyHex: null, reason: 'no-history' };
  const hit = keys.find((k) => k.hex === lc(recKeyHex));
  if (!hit) return { result: FAIL, detail: 'signing key differs from the published key', keyHex: current.public_key_hex, reason: 'not-in-history' };
  if (hit.toSec === null) return { result: UNAVAILABLE, detail: MISCONFIGURED, keyHex: null, reason: 'history-misconfigured' };
  const inside = typeof timestamp === 'number' && Number.isFinite(timestamp) && timestamp >= hit.fromSec && timestamp <= hit.toSec;
  if (!inside) return { result: FAIL, detail: OUTSIDE_WINDOW, keyHex: hit.hex, reason: 'outside-window' };
  return { result: PASS, detail: `historical key ${hit.key_id}`, keyHex: hit.hex, reason: 'historical-key' };
}
