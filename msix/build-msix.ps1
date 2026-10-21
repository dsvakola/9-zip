# Builds Release\9-zip_<version>_x64.msix from the release build of 9-zip.
#   1. npx tauri build --bundles nsis   (makes src-tauri\target\release\9-zip.exe)
#   2. powershell -ExecutionPolicy Bypass -File msix\build-msix.ps1
# The package is unsigned: the Microsoft Store signs it on submission.

$ErrorActionPreference = "Stop"
$root = Split-Path $PSScriptRoot -Parent
$sdk = Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin" -Directory |
    Where-Object { $_.Name -match '^10\.' -and (Test-Path "$($_.FullName)\x64\makeappx.exe") } |
    Sort-Object { [version]$_.Name } | Select-Object -Last 1
if (-not $sdk) { throw "Windows SDK (makeappx.exe) not found." }
$makeappx = "$($sdk.FullName)\x64\makeappx.exe"
$makepri = "$($sdk.FullName)\x64\makepri.exe"

# Version: tauri.conf.json is the single source; MSIX needs four parts.
$conf = Get-Content "$root\src-tauri\tauri.conf.json" -Raw | ConvertFrom-Json
$version = "$($conf.version).0"
$identity = Get-Content "$PSScriptRoot\store-identity.json" -Raw | ConvertFrom-Json

$exe = "$root\src-tauri\target\release\9-zip.exe"
if (-not (Test-Path $exe)) { throw "Build 9-zip first: $exe is missing." }

# The Explorer menu DLL (Windows 11 right-click menu).
Push-Location "$root\src-tauri\shell-ext"
cargo build --release
$built = $LASTEXITCODE
Pop-Location
if ($built -ne 0) { throw "Building the Explorer menu DLL failed." }
$shellDll = "$root\src-tauri\shell-ext\target\release\ninezip_shell.dll"

# Stage the package contents.
$stage = "$root\src-tauri\target\msix-stage"
if (Test-Path $stage) { Remove-Item $stage -Recurse -Force }
New-Item -ItemType Directory -Force "$stage\Assets" | Out-Null
Copy-Item $exe "$stage\9-zip.exe"
Copy-Item $shellDll "$stage\ninezip_shell.dll"
Copy-Item "$PSScriptRoot\Assets\*" "$stage\Assets\"
foreach ($f in "NOTICE", "README.md") { Copy-Item "$root\$f" "$stage\$f" }

$manifest = (Get-Content "$PSScriptRoot\AppxManifest.template.xml" -Raw).
    Replace("{{IDENTITY_NAME}}", $identity.identityName).
    Replace("{{PUBLISHER}}", $identity.publisher).
    Replace("{{PUBLISHER_DISPLAY_NAME}}", $identity.publisherDisplayName).
    Replace("{{VERSION}}", $version)
[IO.File]::WriteAllText("$stage\AppxManifest.xml", $manifest, (New-Object Text.UTF8Encoding $false))

# resources.pri maps the scale-/targetsize- image variants.
Push-Location $stage
& $makepri createconfig /cf "$stage\..\priconfig.xml" /dq en-US /pv 10.0.0 /o | Out-Null
# One package, not a bundle: keep every image variant in a single resources.pri.
[xml]$pri = Get-Content "$stage\..\priconfig.xml"
$pri.SelectNodes("//packaging") | ForEach-Object { $_.ParentNode.RemoveChild($_) | Out-Null }
$pri.Save("$stage\..\priconfig.xml")
& $makepri new /pr $stage /cf "$stage\..\priconfig.xml" /mn "$stage\AppxManifest.xml" /of "$stage\resources.pri" /o | Out-Null
if ($LASTEXITCODE -ne 0) { Pop-Location; throw "makepri failed." }
Pop-Location

$out = "$root\Release\9-zip_${version}_x64.msix"
New-Item -ItemType Directory -Force "$root\Release" | Out-Null
& $makeappx pack /d $stage /p $out /o
if ($LASTEXITCODE -ne 0) { throw "makeappx failed." }
Write-Host "Built $out"
