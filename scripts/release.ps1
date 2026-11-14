# Publie une nouvelle version : bump -> build -> SHA-256 -> release GitHub -> version.json
# Usage : npm run release -- -Version 1.0.1 -Notes "Correctifs"
param(
  [Parameter(Mandatory = $true)][string]$Version,
  [string]$Notes = ''
)
$ErrorActionPreference = 'Stop'
Set-Location (Split-Path $PSScriptRoot -Parent)
if ($Version -notmatch '^\d+\.\d+\.\d+$') { throw "Version invalide (ex: 1.0.1)" }

# outils (même si le PATH du terminal n'est pas encore rafraîchi)
$env:Path = "$env:USERPROFILE\.cargo\bin;C:\Program Files\GitHub CLI;$env:Path"
if (-not (Get-Command gh -ErrorAction SilentlyContinue)) { throw "GitHub CLI absent : winget install GitHub.cli" }
gh auth status *> $null
if ($LASTEXITCODE -ne 0) { throw "GitHub CLI non connecté : gh auth login" }

function Set-First($file, $pattern, $replacement) {
  $c = [IO.File]::ReadAllText($file)
  $c = ([regex]$pattern).Replace($c, $replacement, 1)
  [IO.File]::WriteAllText($file, $c)
}

# 1. bump de version partout
Set-First 'src-tauri/tauri.conf.json' '"version":\s*"[^"]*"' "`"version`": `"$Version`""
Set-First 'src-tauri/Cargo.toml' '(?m)^version = "[^"]*"' "version = `"$Version`""
Set-First 'package.json' '"version":\s*"[^"]*"' "`"version`": `"$Version`""

# 2. build release
npm run build
if ($LASTEXITCODE -ne 0) { throw "Build échoué" }

# 3. artefacts
New-Item -ItemType Directory -Force dist | Out-Null
Copy-Item 'src-tauri/target/release/rez.exe' 'dist/REZ.exe' -Force
$setup = Get-ChildItem 'src-tauri/target/release/bundle/nsis/*.exe' | Sort-Object LastWriteTime -Descending | Select-Object -First 1
$sha = (Get-FileHash 'dist/REZ.exe' -Algorithm SHA256).Hash.ToLower()

$repo = (gh repo view --json nameWithOwner -q .nameWithOwner).Trim()
$tag = "v$Version"
$url = "https://github.com/$repo/releases/download/$tag/REZ.exe"

# 4. commit + release GitHub (asset dispo AVANT de publier version.json)
git add -A
$coauthor = 'Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>'
git commit -m "release $tag" -m $coauthor | Out-Null
git push origin main
$assets = @('dist/REZ.exe')
if ($setup) { Copy-Item $setup.FullName "dist/REZ-setup-$Version.exe" -Force; $assets += "dist/REZ-setup-$Version.exe" }
gh release create $tag @assets --title "REZ $tag" --notes ($(if ($Notes) { $Notes } else { "REZ $tag" })) --target main

# 5. version.json -> déclenche la MAJ chez les utilisateurs
$json = [ordered]@{ version = $Version; url = $url; sha256 = $sha; notes = $Notes } | ConvertTo-Json
[IO.File]::WriteAllText((Join-Path (Get-Location) 'version.json'), $json + "`n")
git add version.json
git commit -m "version.json -> $tag" -m $coauthor | Out-Null
git push origin main

Write-Host "Publié : $tag"
Write-Host "EXE    : $url"
Write-Host "SHA256 : $sha"
