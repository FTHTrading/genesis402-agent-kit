#!/usr/bin/env node
// x402scan origin registration with Sign-In-With-X (CAIP-122 / EIP-4361), signature supplied by Kevan.
//
//   step 1  node tools/x402scan-register.mjs message 0xYourWalletAddress
//           -> fetches a fresh SIWX challenge (valid 5 min) and prints the exact EIP-4361 message to sign
//              (eip191 / personal_sign) plus the challenge JSON to keep for step 2.
//   step 2  node tools/x402scan-register.mjs submit 0xYourWalletAddress 0x<signature> '<challenge json from step 1>'
//           -> sends the SIGN-IN-WITH-X header (base64 JSON per the x402 sign-in-with-x extension) with the
//              register-origin request and prints the response.
//
// This script never signs and never reads a key. Sign with the wallet that owns the origin's payTo.
import { argv, exit } from "node:process";

const ORIGIN = process.env.X402_ORIGIN || "https://twin.unykorn.org";
const URL = "https://www.x402scan.com/api/x402/registry/register-origin";

function siweMessage(info, address) {
  // EIP-4361 layout, as reconstructed by the verifier (x402 sign-in-with-x spec, "Message Format").
  const lines = [
    `${info.domain} wants you to sign in with your Ethereum account:`,
    address,
    "",
    info.statement || "",
    "",
    `URI: ${info.uri}`,
    `Version: ${info.version}`,
    `Chain ID: ${String(info.chainId).replace("eip155:", "")}`,
    `Nonce: ${info.nonce}`,
    `Issued At: ${info.issuedAt}`,
  ];
  if (info.expirationTime) lines.push(`Expiration Time: ${info.expirationTime}`);
  if (info.notBefore) lines.push(`Not Before: ${info.notBefore}`);
  if (info.requestId) lines.push(`Request ID: ${info.requestId}`);
  if (Array.isArray(info.resources) && info.resources.length) { lines.push("Resources:"); for (const r of info.resources) lines.push(`- ${r}`); }
  return lines.join("\n");
}

async function challenge() {
  const r = await fetch(URL, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ origin: ORIGIN }) });
  const j = await r.json();
  const info = j?.extensions?.["sign-in-with-x"]?.info;
  if (r.status !== 402 || !info) { console.error("unexpected response", r.status, JSON.stringify(j).slice(0, 400)); exit(1); }
  return info;
}

const [mode, address, signature, challengeJson] = argv.slice(2);
if (mode === "message" && address) {
  const info = await challenge();
  console.log("=== SIWX message to sign with", address, "(eip191 / personal_sign) — expires", info.expirationTime, "===\n");
  console.log(siweMessage(info, address));
  console.log("\n=== keep this challenge JSON for step 2 ===");
  console.log(JSON.stringify(info));
} else if (mode === "submit" && address && signature && challengeJson) {
  const info = JSON.parse(challengeJson);
  const proof = { info: { ...info, address, type: "eip191" }, signatureScheme: "eip191", signature };
  const header = Buffer.from(JSON.stringify(proof)).toString("base64");
  const r = await fetch(URL, { method: "POST", headers: { "content-type": "application/json", "SIGN-IN-WITH-X": header }, body: JSON.stringify({ origin: ORIGIN }) });
  console.log(r.status, (await r.text()).slice(0, 1500));
} else {
  console.error("usage: message <address> | submit <address> <0xsignature> '<challenge json>'"); exit(2);
}
