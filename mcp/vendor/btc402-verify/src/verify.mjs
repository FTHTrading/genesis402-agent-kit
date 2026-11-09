import { createHash, createPublicKey, verify } from 'node:crypto';
import { canonicalBytes } from './canonical.mjs';
import { resolveSigningKey } from './key-history-core.mjs';

const PASS = 'PASS', FAIL = 'FAIL', UNAVAILABLE = 'UNAVAILABLE';
const check = (name, result, detail, reason) => ({ name, result, detail, ...(reason ? { reason } : {}) });

const SPKI_ED25519_PREFIX = Buffer.from('302a300506032b6570032100', 'hex');

// Build the key from the raw 32 bytes (no PEM needed). Import or algorithm errors are UNAVAILABLE; only verify() === false is FAIL.
function signatureCheck(bytes, sigHex, keyHex) {
  let pub;
  try {
    if (!/^[0-9a-fA-F]{64}$/.test(String(keyHex))) throw new Error('public_key_hex is not 32 bytes of hex');
    pub = createPublicKey({ key: Buffer.concat([SPKI_ED25519_PREFIX, Buffer.from(keyHex, 'hex')]), format: 'der', type: 'spki' });
  } catch (e) {
    return check('signature', UNAVAILABLE, `public key could not be loaded: ${e.message}`);
  }
  if (!/^[0-9a-fA-F]{128}$/.test(String(sigHex))) return check('signature', FAIL, 'sig_hex is not a 64-byte signature');
  let ok;
  try {
    ok = verify(null, bytes, pub, Buffer.from(sigHex, 'hex'));
  } catch (e) {
    return check('signature', UNAVAILABLE, `Ed25519 verify unavailable: ${e.message}`);
  }
  return check('signature', ok === true ? PASS : FAIL,
    ok === true ? 'Ed25519 signature valid over canonical bytes' : 'Ed25519 signature does not verify');
}

// Verify a btc402 attestation against the published key, or against a retired key listed in the key history (/attestations/keys)
// whose validity window contains the record's signed execution.timestamp. Pure: no network, no I/O.
// history: the parsed /attestations/keys body, or null/undefined when it could not be loaded (then only the current key can verify).
// opts is kept for callers; a retired key's window always has an end date.
export function verifyAttestation(rec, key, history, opts = {}) {
  const checks = [];
  const sig = rec?.ed25519;
  const hasSig = !!(sig && sig.sig_hex && sig.public_key_hex);
  checks.push(check('has_ed25519', hasSig ? PASS : FAIL,
    hasSig ? 'record carries an Ed25519 signature' : 'record has no Ed25519 signature (pre-v2 records cannot be verified)'));

  const bytes = canonicalBytes(rec ?? {});
  const digest = createHash('sha256').update(bytes).digest('hex');
  checks.push(check('digest', digest === rec?.digest ? PASS : FAIL,
    digest === rec?.digest ? 'sha256(canonical bytes) equals digest' : `computed ${digest}, record says ${rec?.digest}`));

  if (!hasSig) {
    checks.push(check('key_matches_published', UNAVAILABLE, 'no signing key in record'));
    checks.push(check('signature', UNAVAILABLE, 'no signature in record'));
  } else {
    const k = resolveSigningKey({ recKeyHex: sig.public_key_hex, timestamp: rec?.execution?.timestamp, current: key, history });
    checks.push(check('key_matches_published', k.result, k.detail, k.reason));
    checks.push(k.keyHex ? signatureCheck(bytes, sig.sig_hex, k.keyHex) : check('signature', UNAVAILABLE, k.detail, k.reason));
  }
  return { ok: checks.every((c) => c.result === PASS), checks };
}
