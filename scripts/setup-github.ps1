# Configure le dépôt GitHub (une seule fois).
# Usage : npm run setup-github -- -User TonPseudoGitHub [-Repo REZ]
param(
  [Parameter(Mandatory = $true)][string]$User,
  [string]$Repo = 'REZ'
)
$ErrorActionPreference = 'Stop'
Set-Location (Split-Path $PSScriptRoot -Parent)

if (-not (Get-Command gh -ErrorAction SilentlyContinue)) {
  throw "GitHub CLI absent : winget install GitHub.cli  puis  gh auth login"
}

# URL version.json dans le code de l'updater
$f = 'src-tauri/src/updater.rs'
$c = [IO.File]::ReadAllText($f)
$c = [regex]::Replace($c, 'https://raw\.githubusercontent\.com/[^/]+/[^/]+/main/version\.json', "https://raw.githubusercontent.com/$User/$Repo/main/version.json")
[IO.File]::WriteAllText($f, $c)
Write-Host "updater.rs -> https://raw.githubusercontent.com/$User/$Repo/main/version.json"

if (-not (Test-Path .git)) { git init -b main | Out-Null }
git add -A
git commit -m "REZ: initial" | Out-Null

# Dépôt public obligatoire : version.json et REZ.exe doivent être téléchargeables sans compte
gh repo create "$User/$Repo" --public --source . --remote origin --push
Write-Host "OK -> https://github.com/$User/$Repo"
