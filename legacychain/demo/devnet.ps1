# LegacyChain three-validator devnet for PowerShell 7+ (Windows, macOS, Linux).
# Same flows as devnet.sh: money with a sealed memo, a sealed contract, an
# agent with escrow, an RWA note, x402 + MPP deposits, and a withdrawal.
#
# Usage (from the legacychain folder):   ./demo/devnet.ps1
#        optimized build:                ./demo/devnet.ps1 -Release
param([switch]$Release)

$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $true
Set-Location (Join-Path $PSScriptRoot '..')

if ($Release) { cargo build --release -q; $Profile_ = 'release' } else { cargo build -q; $Profile_ = 'debug' }
$Ext = if ($IsWindows) { '.exe' } else { '' }
$Bin = Join-Path 'target' $Profile_
$Lc = Join-Path $Bin "lc$Ext"
$NodeBin = Join-Path $Bin "legacychain-node$Ext"
$W = Join-Path 'demo' '.devnet'
if (Test-Path $W) { Remove-Item -Recurse -Force $W }
$Keys = Join-Path $W 'keys'
New-Item -ItemType Directory -Force $Keys | Out-Null

foreach ($h in 'hq', 'london', 'alaska', 'stravin') { & $Lc keygen --handle $h --out-dir $Keys | Out-Null }
'LegacyChain devnet charter: members, validators, and terms are fixed at genesis.' | Set-Content (Join-Path $W 'charter.txt')
$Genesis = Join-Path $W 'genesis.json'
& $Lc genesis --chain-id legacychain-devnet --mode devnet `
    --member (Join-Path $Keys 'hq.member.json') --member (Join-Path $Keys 'london.member.json') `
    --member (Join-Path $Keys 'alaska.member.json') --member (Join-Path $Keys 'stravin.member.json') `
    --role 'hq=validator,admin,bridge,issuer' --role 'london=validator' --role 'alaska=validator' `
    --pay-to 0x000000000000000000000000000000000000dEaD `
    --allocate hq=1000000 --charter (Join-Path $W 'charter.txt') --out $Genesis

$Sites = @(
    @{ Handle = 'hq'; Port = 7402 },
    @{ Handle = 'london'; Port = 7403 },
    @{ Handle = 'alaska'; Port = 7404 }
)
$Procs = @()

function Step($text) { Write-Host "`n== $text" -ForegroundColor Cyan }

function Invoke-As([string]$Handle, [int]$Port) {
    $env:LC_KEY = Join-Path $Keys "$Handle.key.json"
    $env:LC_NODE = "http://127.0.0.1:$Port"
    & $Lc @args
}

try {
    $env:RUST_LOG = 'warn'
    foreach ($s in $Sites) {
        $peers = ($Sites | Where-Object { $_.Port -ne $s.Port } | ForEach-Object { "http://127.0.0.1:$($_.Port)" }) -join ','
        $nodeArgs = @(
            '--genesis', $Genesis, '--key', (Join-Path $Keys "$($s.Handle).key.json"),
            '--data', (Join-Path $W "data-$($s.Handle)"), '--listen', "127.0.0.1:$($s.Port)",
            '--peer', $peers, '--public-url', "http://127.0.0.1:$($s.Port)", '--simulate-settlement'
        )
        $Procs += Start-Process -FilePath $NodeBin -ArgumentList $nodeArgs -NoNewWindow -PassThru `
            -RedirectStandardOutput (Join-Path $W "$($s.Handle).out.log") -RedirectStandardError (Join-Path $W "$($s.Handle).log")
    }
    foreach ($s in $Sites) {
        $up = $false
        for ($i = 0; $i -lt 100 -and -not $up; $i++) {
            try { Invoke-RestMethod "http://127.0.0.1:$($s.Port)/v1/health" | Out-Null; $up = $true } catch { Start-Sleep -Milliseconds 100 }
        }
        if (-not $up) { throw "node $($s.Handle) did not start; see $W\$($s.Handle).log" }
    }

    Step 'Money: HQ sends 250,000 to London with a sealed invoice memo'
    Invoke-As hq 7402 send --to london --amount 250000 --memo 'Invoice LDN-0042: bridge loan tranche 1, due 30 days'

    Step 'Contract: HQ seals a term sheet to London and Alaska'
    $TermSheet = Join-Path $W 'termsheet.txt'
    "TERM SHEET`nFacility: 2,000,000 senior secured`nRate: 8.25%`nGoverning law: England & Wales" | Set-Content $TermSheet
    Invoke-As hq 7402 seal --to 'london,alaska' --kind contract --title 'Facility term sheet' --file $TermSheet

    Step 'Agent: HQ sends an agent to Alaska with 5,000 in escrow'
    $out = Invoke-As hq 7402 dispatch --to alaska --escrow 5000 --task 'Inspect the Anchorage warehouse receipts for lot 7 and accept if they match the manifest.'
    $out | Write-Host
    $Dispatch = (($out | Select-String 'dispatch id (\w+)').Matches[0].Groups[1].Value)

    Step 'London opens its inbox (memo + term sheet) via the London node'
    Invoke-As london 7403 inbox

    Step 'Alaska opens its inbox via the Alaska node, then accepts the agent task'
    Invoke-As alaska 7404 inbox
    Invoke-As alaska 7404 respond --dispatch $Dispatch --result 'Receipts match manifest. Lot 7 verified.'

    Step 'Stravin is a member but was not addressed: nothing opens'
    Invoke-As stravin 7402 inbox

    Step 'RWA: HQ issues a note; the offering document is sealed to London'
    $Offering = Join-Path $W 'offering.txt'
    "OFFERING MEMORANDUM`nLondon Senior Note 2031, 1,000 units" | Set-Content $Offering
    Invoke-As hq 7402 rwa-issue --asset-id LDN-NOTE-2031 --name 'London Senior Note 2031' --supply 1000 --document $Offering --holders london
    Invoke-As hq 7402 rwa-send --asset-id LDN-NOTE-2031 --to london --amount 250

    # Well-known test key; holds no funds. Signatures are real EIP-3009; settlement is simulated on devnet.
    $TestEvmKey = '4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318'
    Step 'Real-money ingress: x402 deposit for Stravin'
    Invoke-As stravin 7402 deposit --amount 25 --evm-key $TestEvmKey
    Step 'Same thing over MPP (WWW-Authenticate: Payment / Authorization: Payment)'
    Invoke-As stravin 7402 deposit --amount 10 --evm-key $TestEvmKey --mpp

    Step 'Redemption: Stravin withdraws 5 back to a Base address'
    Invoke-As stravin 7402 withdraw --amount 5 --payout-to 0x2c7536E3605D9C16a7a3D7b1898e529396a65c23

    Step 'Balances (each read is a signed request; nothing is public)'
    foreach ($p in @(@('hq', 7402), @('london', 7403), @('alaska', 7404), @('stravin', 7402))) {
        $me = (Invoke-As $p[0] $p[1] me) -join "`n" | ConvertFrom-Json
        $rwa = ($me.rwa | ForEach-Object { "$($_.asset_id)=$($_.units)" }) -join ', '
        '{0,-8} {1} {2} | rwa: [{3}] | height {4}' -f $p[0], $me.balance, $me.symbol, $rwa, $me.height | Write-Host
    }

    Step 'Anonymous read of an account is refused'
    try { Invoke-RestMethod 'http://127.0.0.1:7402/v1/me' | Out-Null; throw 'anonymous read was allowed' }
    catch { if ($_.Exception.Message -eq 'anonymous read was allowed') { throw }; Write-Host "refused: $($_.ErrorDetails.Message)" }

    Step 'All three validators agree'
    foreach ($s in $Sites) { Invoke-RestMethod "http://127.0.0.1:$($s.Port)/v1/health" | ConvertTo-Json -Compress | Write-Host }
}
finally {
    foreach ($p in $Procs) { if ($p -and -not $p.HasExited) { Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue } }
    Remove-Item Env:LC_KEY, Env:LC_NODE, Env:RUST_LOG -ErrorAction SilentlyContinue
}
