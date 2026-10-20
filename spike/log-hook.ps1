# Spike only. Appends one line per Claude hook call:
# time, the CLAUDE_DECK_SESSION variable, and the raw JSON from standard input.
$raw = [Console]::In.ReadToEnd()
$line = '{0} deck={1} {2}' -f (Get-Date -Format o), $env:CLAUDE_DECK_SESSION, ($raw -replace "`r?`n", ' ')
Add-Content -Path (Join-Path $PSScriptRoot 'hooks.log') -Value $line -Encoding utf8
