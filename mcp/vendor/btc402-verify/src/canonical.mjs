import { canonicalJson, EXCLUDED_FIELDS } from './canonical-core.mjs';

export { canonicalJson, EXCLUDED_FIELDS };

// The exact bytes the gate signs: the record minus EXCLUDED_FIELDS, canonicalised, UTF-8.
export function canonicalBytes(rec) {
  const body = Object.fromEntries(Object.entries(rec).filter(([k]) => !EXCLUDED_FIELDS.includes(k)));
  return Buffer.from(canonicalJson(body), 'utf8');
}
