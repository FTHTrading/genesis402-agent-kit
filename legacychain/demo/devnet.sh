#!/usr/bin/env bash
# LegacyChain three-validator devnet: HQ, London, Alaska on one machine.
#
# Shows, end to end:
#   money        HQ -> London transfer with a sealed memo (invoice text only London can read)
#   contracts    a sealed contract to London + Alaska; an outsider member sees ciphertext only
#   agents       HQ sends an agent to Alaska with 5,000 in escrow; Alaska accepts and is paid
#   RWA          a note issued with its offering document sealed to the holders
#   real-money   x402 and MPP deposits (real EIP-3009 signatures, settlement simulated on devnet)
#   redemption   a withdrawal back to a Base address
#
# Usage: demo/devnet.sh            (PROFILE=release for an optimized build)
set -euo pipefail
cd "$(dirname "$0")/.."
PROFILE="${PROFILE:-debug}"
if [ "$PROFILE" = release ]; then cargo build --release -q; else cargo build -q; fi
BIN="target/$PROFILE"
LC="$BIN/lc"
W="demo/.devnet"
rm -rf "$W" && mkdir -p "$W/keys"

for h in hq london alaska stravin; do "$LC" keygen --handle "$h" --out-dir "$W/keys" >/dev/null; done
echo "LegacyChain devnet charter: members, validators, and terms are fixed at genesis." > "$W/charter.txt"
"$LC" genesis --chain-id legacychain-devnet --mode devnet \
  --member "$W/keys/hq.member.json" --member "$W/keys/london.member.json" \
  --member "$W/keys/alaska.member.json" --member "$W/keys/stravin.member.json" \
  --role hq=validator,admin,bridge,issuer --role london=validator --role alaska=validator \
  --pay-to 0x000000000000000000000000000000000000dEaD \
  --allocate hq=1000000 --charter "$W/charter.txt" --out "$W/genesis.json"

PIDS=()
cleanup() { for p in "${PIDS[@]}"; do kill "$p" 2>/dev/null || true; done; }
trap cleanup EXIT
start() { # handle port peers...
  local h=$1 port=$2; shift 2
  local peers; peers=$(IFS=,; echo "$*")
  RUST_LOG=warn "$BIN/legacychain-node" --genesis "$W/genesis.json" --key "$W/keys/$h.key.json" \
    --data "$W/data-$h" --listen "127.0.0.1:$port" --peer "$peers" \
    --public-url "http://127.0.0.1:$port" --simulate-settlement >"$W/$h.log" 2>&1 &
  PIDS+=($!)
}
start hq 7402 http://127.0.0.1:7403 http://127.0.0.1:7404
start london 7403 http://127.0.0.1:7402 http://127.0.0.1:7404
start alaska 7404 http://127.0.0.1:7402 http://127.0.0.1:7403
for port in 7402 7403 7404; do
  for _ in $(seq 1 50); do curl -sf "http://127.0.0.1:$port/v1/health" >/dev/null && break; sleep 0.1; done
done

as() { local h=$1 port=$2; shift 2; LC_KEY="$W/keys/$h.key.json" LC_NODE="http://127.0.0.1:$port" "$LC" "$@"; }
step() { printf '\n\033[1m== %s\033[0m\n' "$*"; }

step "Money: HQ sends 250,000 to London with a sealed invoice memo"
as hq 7402 send --to london --amount 250000 --memo "Invoice LDN-0042: bridge loan tranche 1, due 30 days"

step "Contract: HQ seals a term sheet to London and Alaska"
printf 'TERM SHEET\nFacility: 2,000,000 senior secured\nRate: 8.25%%\nGoverning law: England & Wales\n' > "$W/termsheet.txt"
as hq 7402 seal --to london,alaska --kind contract --title "Facility term sheet" --file "$W/termsheet.txt"

step "Agent: HQ sends an agent to Alaska with 5,000 in escrow"
DISPATCH=$(as hq 7402 dispatch --to alaska --escrow 5000 --task "Inspect the Anchorage warehouse receipts for lot 7 and accept if they match the manifest." | tee /dev/stderr | awk '/dispatch id/{print $3}')

step "London opens its inbox (memo + term sheet) via the London node"
as london 7403 inbox

step "Alaska opens its inbox via the Alaska node, then accepts the agent task"
as alaska 7404 inbox
as alaska 7404 respond --dispatch "$DISPATCH" --result "Receipts match manifest. Lot 7 verified."

step "Stravin is a member but was not addressed: nothing opens"
as stravin 7402 inbox

step "RWA: HQ issues a note; the offering document is sealed to London"
printf 'OFFERING MEMORANDUM\nLondon Senior Note 2031, 1,000 units\n' > "$W/offering.txt"
as hq 7402 rwa-issue --asset-id LDN-NOTE-2031 --name "London Senior Note 2031" --supply 1000 --document "$W/offering.txt" --holders london
as hq 7402 rwa-send --asset-id LDN-NOTE-2031 --to london --amount 250

step "Real-money ingress: x402 deposit for Stravin (EIP-3009 signed; settlement simulated on devnet)"
TEST_EVM_KEY=4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318
as stravin 7402 deposit --amount 25 --evm-key "$TEST_EVM_KEY"
step "Same thing over MPP (WWW-Authenticate: Payment / Authorization: Payment)"
as stravin 7402 deposit --amount 10 --evm-key "$TEST_EVM_KEY" --mpp

step "Redemption: Stravin withdraws 5 back to a Base address"
as stravin 7402 withdraw --amount 5 --payout-to 0x2c7536E3605D9C16a7a3D7b1898e529396a65c23

step "Balances (each read is a signed request; nothing is public)"
for spec in "hq 7402" "london 7403" "alaska 7404" "stravin 7402"; do
  set -- $spec
  printf '%-8s ' "$1"; as "$1" "$2" me | python3 -c 'import json,sys; d=json.load(sys.stdin); print(d["balance"], d["symbol"], "| rwa:", [(r["asset_id"], r["units"]) for r in d["rwa"]], "| height", d["height"])'
done

step "Anonymous read of an account is refused"
curl -s http://127.0.0.1:7402/v1/me; echo

step "All three validators agree"
for port in 7402 7403 7404; do curl -s "http://127.0.0.1:$port/v1/health"; echo; done
