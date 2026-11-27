# Proposal: a feedback link inside the product

Status: PROPOSAL. Nothing in this file is implemented. It is a specification for the rail's release lane to pick up later. Written 2026-11-26.

## Why

Today a caller who pays for a call has no pointer to a place where they can say how it went. The only link that reaches every caller is one that travels with the tool description and with the receipt. The text below is the same for every caller. It is never shown to some callers and withheld from others, and it asks for no particular answer.

Feedback form: `https://github.com/FTHTrading/genesis402-agent-kit/issues/new?template=feedback.yml`

## Part 1: one sentence appended to MCP tool descriptions

Append this exact sentence, as the last sentence, to the descriptions of `genesis402_catalog` and `genesis402_receipt` (the two free tools, which every session can reach):

> Feedback, good or bad, is welcome at https://github.com/FTHTrading/genesis402-agent-kit/issues/new?template=feedback.yml (a public GitHub issue; UnyKorn LLC operates this server).

Rules for whoever implements it:

- The sentence is identical for every caller and every tool it is added to. No variant by payer, amount, result or outcome.
- Do not add it to paid tool descriptions in the first release. Paid tool descriptions are scored by directories for clarity, and a link in each of them is noise. Reconsider after one release.
- Do not instruct the model to ask the user for feedback, and do not add any wording about ratings, stars or scores.
- Keep it out of the server `instructions` block, which is about how to use the tools.
- Add a test that the sentence appears in exactly those two descriptions and nowhere else.

## Part 2: one optional field in the signed receipt

Add one optional string field to the receipt object:

```json
"feedback_url": "https://github.com/FTHTrading/genesis402-agent-kit/issues/new?template=feedback.yml"
```

Rules for whoever implements it:

- The field is part of the receipt before it is signed, so it is covered by the Ed25519 signature like every other field (the signature is over the canonical JSON of the receipt without its `signature` field).
- It is a constant. The same value goes into every receipt issued after the change, for every payer and every outcome.
- Receipts already issued are never altered or re-signed. A verifier must accept a receipt with or without the field. The current signing domain `genesis402-payment-receipt/1` signs whatever fields are present, so adding an optional field does not need a new domain; confirm this against the verifier at `/receipts/verify` before release, and add a test with one old receipt and one new receipt.
- The public feed (`/receipts`) may omit the field, since it is the same on every row. `/receipts/sample` should show it once the change is live, so that `examples/quickstart.mjs` in this repository keeps verifying the sample without changes.
- The field carries a link only. No text, no score request, no payer-specific token or tracking parameter.

## What this proposal does not do

- It does not write, request in bulk or pre-fill any review, rating or feedback entry.
- It does not offer anything in return for feedback.
- It does not change prices, `payTo`, the 402 response or any existing receipt field.

## How to tell it worked

- `tools/list` on `https://twin.unykorn.org/mcp` shows the sentence on the two free tools.
- `GET https://twin.unykorn.org/receipts/sample` shows `feedback_url`, and `POST /receipts/verify` with that sample returns `"valid": true`.
- `node examples/quickstart.mjs` still ends with a valid signature and a rejected altered copy.
- Count, weekly, the issues opened with the `feedback` label by authors who are not UnyKorn staff. That number is the result; it may stay at zero.
