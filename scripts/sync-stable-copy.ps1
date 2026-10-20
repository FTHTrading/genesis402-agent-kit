# Refresh the stable copy the Windows task 'UnyKorn-Smoke-Hourly' runs (C:\Users\Kevan\scripts\smoke\listing-watch\).
# Run after every change to scripts/listing-watch.mjs or scripts/lib/ on chore/listing-automation (or main once merged).
# A GitHub workflow cannot do this: the stable copy is on Kevan's machine. Run from the repo root.
$ErrorActionPreference = 'Stop'
$dest = 'C:\Users\Kevan\scripts\smoke\listing-watch'
New-Item -ItemType Directory -Force -Path "$dest\lib" | Out-Null
Copy-Item -Force scripts\listing-watch.mjs $dest
Copy-Item -Force scripts\lib\*.mjs "$dest\lib"
Get-FileHash "$dest\listing-watch.mjs", "$dest\lib\listing-parse.mjs" -Algorithm SHA256 | Format-Table Hash, Path -AutoSize
