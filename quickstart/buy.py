"""Buy one Genesis402 call from Python.

  pip install "x402[httpx,evm]" eth-account xrpl-py
  python buy.py genesis-sim '{"n":20,"epochs":20}'                       # quote only
  PAYER_KEY=0x... LIVE=1 python buy.py genesis-sim '{"n":20,"epochs":20}' # $0.25 USDC on Base
  XRPL_SEED=s... LIVE=1 RAIL=xrpl python buy.py genesis-sim '{"n":20}'   # 0.05 XRP on XRPL
"""
import asyncio, base64, json, os, sys

import httpx

ORIGIN = os.getenv("ORIGIN", "https://twin.unykorn.org")
task = sys.argv[1] if len(sys.argv) > 1 else "genesis-sim"
params = json.loads(sys.argv[2]) if len(sys.argv) > 2 else {}
URL = f"{ORIGIN}/{task}"
BODY = {"params": params}


async def main() -> None:
    async with httpx.AsyncClient(timeout=60) as http:
        probe = await http.post(URL, json=BODY)
    if probe.status_code != 402:
        print(probe.status_code, probe.text)
        return
    challenge = json.loads(base64.b64decode(probe.headers["payment-required"]))
    base = next(a for a in challenge["accepts"] if a["network"] == "eip155:8453")
    xrp = next((a for a in challenge["accepts"] if a["network"] == "xrpl:mainnet"), None)
    print("402 challenge:", {"base_usdc": int(base["amount"]) / 1e6, "xrpl_xrp": xrp and xrp["price"]})
    if os.getenv("LIVE") != "1":
        print("Quote only. Set LIVE=1 plus PAYER_KEY (Base) or XRPL_SEED with RAIL=xrpl to pay.")
        return

    if os.getenv("RAIL") == "xrpl":
        # XRPL is pay-first: submit the payment, then present the validated tx hash.
        from xrpl.asyncio.clients import AsyncWebsocketClient
        from xrpl.asyncio.transaction import submit_and_wait
        from xrpl.models.transactions import Memo, Payment
        from xrpl.utils import xrp_to_drops
        from xrpl.wallet import Wallet

        wallet = Wallet.from_seed(os.environ["XRPL_SEED"])
        async with AsyncWebsocketClient("wss://xrplcluster.com") as client:
            # memo = sha256(nonce) binds the payment to this challenge
            memo = Memo(memo_data=xrp["extra"]["memo_sha256"].upper())
            tx = await submit_and_wait(Payment(account=wallet.address, destination=xrp["payTo"], amount=xrp["amount"], memos=[memo]), client, wallet)
        tx_hash = tx.result["hash"]
        print("XRPL payment validated:", tx_hash)
        async with httpx.AsyncClient(timeout=60) as http:
            res = await http.post(URL, json=BODY, headers={"X-PAYMENT": json.dumps({"network": "xrpl:mainnet", "txHash": tx_hash, "nonce": xrp["extra"]["nonce"]})})
        print(res.status_code, res.text)
        return

    # Base: the official x402 client signs EIP-3009 and retries with PAYMENT-SIGNATURE.
    from eth_account import Account
    from x402 import x402Client
    from x402.http.clients import x402HttpxClient
    from x402.mechanisms.evm import EthAccountSigner
    from x402.mechanisms.evm.exact.register import register_exact_evm_client

    client = x402Client().set_spend_controls({"max_amount_per_payment": "$0.25"})
    register_exact_evm_client(client, EthAccountSigner(Account.from_key(os.environ["PAYER_KEY"])))
    async with x402HttpxClient(client) as http:
        res = await http.post(URL, json=BODY)
        await res.aread()
    print(res.status_code, res.text)


asyncio.run(main())
