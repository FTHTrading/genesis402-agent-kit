// Browser-safe canonical JSON (no Node imports, no Buffer).
// Matches Python json.dumps(sort_keys=True, separators=(',',':'), ensure_ascii=True) as used by
// erc8004.canonical_bytes in the btc402 gate: every char >= U+007F is escaped as lowercase \uXXXX
// (astral chars come out as surrogate pairs because JS strings are UTF-16).
export const EXCLUDED_FIELDS = Object.freeze(['digest', 'ed25519', 'internal_hmac_sha256', 'signature']);

const esc = (s) =>
  JSON.stringify(s).replace(/[\u007f-\uffff]/g, (ch) => '\\u' + ch.charCodeAt(0).toString(16).padStart(4, '0'));

export function canonicalJson(v) {
  if (Array.isArray(v)) return '[' + v.map(canonicalJson).join(',') + ']';
  if (v && typeof v === 'object') {
    return '{' + Object.keys(v).sort().map((k) => esc(k) + ':' + canonicalJson(v[k])).join(',') + '}';
  }
  return typeof v === 'string' ? esc(v) : JSON.stringify(v);
}
