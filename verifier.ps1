<#
.SYNOPSIS
  Verifie que les modifications n'ont pas casse le format des fichiers Zortrax.

.DESCRIPTION
  1. cargo test --workspace --release : tous les tests (format de reference, matrice de reglages,
     allers-retours, import G-code, supports...).
  2. Regenere des fichiers .zcodex2 depuis des STL d'exemple (avec et sans supports en arbre) et les passe
     a `ztool check` (conteneur + format Z-Suite 3.7 + flux de commandes).
  3. Verifie les exemples livres et un G-code d'integration.
  4. Verifie aussi les vrais fichiers Z-Suite du dossier Telechargements (information).

.PARAMETER Matrix
  Nombre de combinaisons de reglages aleatoires (defaut 24 ; essaie 300 avant une livraison).

.PARAMETER Watch
  Relance la verification a chaque modification des sources.

.EXAMPLE
  .\verifier.ps1
  .\verifier.ps1 -Matrix 300
  .\verifier.ps1 -Watch
#>
param(
    [int]$Matrix = 24,
    [switch]$Watch
)

$ErrorActionPreference = "Continue"
$root = Split-Path -Parent $MyInvocation.MyCommand.Path
$env:Path += ";$env:USERPROFILE\.cargo\bin"
$script:fail = 0

function Step($name, [scriptblock]$body) {
    Write-Host ""
    Write-Host "== $name" -ForegroundColor Cyan
    & $body
}

function Ok($msg)  { Write-Host "  OK   $msg" -ForegroundColor Green }
function Bad($msg) { Write-Host "  ECHEC $msg" -ForegroundColor Red; $script:fail++ }

function RunOnce {
    $script:fail = 0
    Push-Location $root
    try {
        Step "1/4 Tests (cargo test --workspace --release, matrice = $Matrix)" {
            $env:ZSLICE_MATRIX = "$Matrix"
            $out = cargo test --workspace --release 2>&1 | Out-String
            $failed = [regex]::Matches($out, "test (\S+) \.\.\. FAILED") | ForEach-Object { $_.Groups[1].Value }
            $summary = [regex]::Matches($out, "test result: (\w+)\. (\d+) passed; (\d+) failed")
            $passed = ($summary | ForEach-Object { [int]$_.Groups[2].Value } | Measure-Object -Sum).Sum
            if ($failed -or $out -match "error(\[E\d+\])?:") {
                Bad "tests en echec : $($failed -join ', ')"
                ($out -split "`n" | Select-String -Pattern "panicked|FORMAT|en échec|ECHEC|error" | Select-Object -First 12) | ForEach-Object { Write-Host "       $_" }
            } else {
                Ok "$passed tests passent (format de reference, matrice, allers-retours...)"
            }
        }

        $ztool = Join-Path $root "target\release\ztool.exe"
        Step "2/4 Fichiers regeneres depuis des STL d'exemple" {
            if (-not (Test-Path $ztool)) { cargo build --release -p zslice --bin ztool 2>&1 | Out-Null }
            $tmp = Join-Path $env:TEMP "verifier-zcodex2"
            New-Item -ItemType Directory -Force $tmp | Out-Null
            $ex = Join-Path $root "zproj-studio\exemples"
            $cases = @(
                @{ stl = "cube30.stl";      flags = @() },
                @{ stl = "T_surplomb.stl";  flags = @("tree", "noraft") },
                @{ stl = "arche.stl";       flags = @("support") }
            )
            foreach ($c in $cases) {
                $stl = Join-Path $ex $c.stl
                if (-not (Test-Path $stl)) { continue }
                $out = Join-Path $tmp ($c.stl + ".zcodex2")
                cargo run --release -q -p zslice --example stl2zcodex2 -- $stl $out @($c.flags) 2>&1 | Out-Null
                $r = & $ztool check $out 2>&1 | Out-String
                if ($LASTEXITCODE -eq 0) { Ok "$($c.stl) $($c.flags -join ' ')" } else { Bad "$($c.stl) : $r" }
            }
        }

        Step "3/4 Exemples livres et G-code d'integration" {
            $files = Get-ChildItem (Join-Path $root "zproj-studio\exemples\*.zcodex2") -ErrorAction SilentlyContinue
            foreach ($f in $files) {
                $r = & $ztool check $f.FullName 2>&1 | Out-String
                if ($LASTEXITCODE -eq 0) { Ok $f.Name } else { Bad "$($f.Name) (regenere-le avec la version actuelle) : $r" }
            }
            $g = Join-Path $root "integrations\exemples\cube30_style_prusaslicer.gcode"
            if (Test-Path $g) {
                $o = Join-Path $env:TEMP "verifier-gcode.zcodex2"
                & $ztool to-zcodex2 $g -o $o -q 2>&1 | Out-Null
                if ($LASTEXITCODE -eq 0) {
                    & $ztool check $o 2>&1 | Out-Null
                    if ($LASTEXITCODE -eq 0) { Ok "G-code d'integration -> .zcodex2" } else { Bad "G-code converti invalide" }
                } else { Bad "conversion du G-code d'integration" }
            }
        }

        Step "4/4 Vrais fichiers Z-Suite (information)" {
            $dl = Join-Path $env:USERPROFILE "Downloads"
            Get-ChildItem "$dl\*.zcodex2" -ErrorAction SilentlyContinue | ForEach-Object {
                $r = & $ztool check $_.FullName 2>&1 | Out-String
                if ($LASTEXITCODE -eq 0) { Ok $_.Name } else { Write-Host "  info  $($_.Name) : fichier non genere par Z-Suite ou a l'ancien format" -ForegroundColor Yellow }
            }
        }
    } finally { Pop-Location }

    Write-Host ""
    if ($script:fail -eq 0) { Write-Host "TOUT EST BON : le format des fichiers est intact." -ForegroundColor Green }
    else { Write-Host "$($script:fail) VERIFICATION(S) EN ECHEC : ne livre pas cette version." -ForegroundColor Red }
    return $script:fail
}

if ($Watch) {
    Write-Host "Surveillance des sources (Ctrl+C pour arreter)..."
    $last = [datetime]::MinValue
    while ($true) {
        $newest = Get-ChildItem $root -Recurse -Include *.rs,*.json,Cargo.toml -ErrorAction SilentlyContinue |
            Where-Object { $_.FullName -notmatch "\\target\\" } | Sort-Object LastWriteTime -Descending | Select-Object -First 1
        if ($newest -and $newest.LastWriteTime -gt $last) {
            $last = $newest.LastWriteTime
            RunOnce | Out-Null
        }
        Start-Sleep -Seconds 3
    }
} else {
    $code = RunOnce
    exit [int]($code -gt 0)
}
