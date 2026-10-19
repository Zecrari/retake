# Installs retake: https://github.com/Zecrari/retake
#
#   irm https://zecrari.github.io/retake/install.ps1 | iex
#
# Environment variables:
#   RETAKE_VERSION      release tag to install, e.g. v0.1.0 (default: latest)
#   RETAKE_INSTALL_DIR  where to put the binary (default: %LOCALAPPDATA%\Programs\retake)
$ErrorActionPreference = 'Stop'

$repo = 'Zecrari/retake'
$version = if ($env:RETAKE_VERSION) { $env:RETAKE_VERSION } else { 'latest' }
$installDir = if ($env:RETAKE_INSTALL_DIR) { $env:RETAKE_INSTALL_DIR } else { Join-Path $env:LOCALAPPDATA 'Programs\retake' }

if (-not [Environment]::Is64BitOperatingSystem) { throw 'retake needs 64-bit Windows.' }
$target = 'x86_64-pc-windows-msvc'
$asset = "retake-$target.zip"
$base = if ($version -eq 'latest') { "https://github.com/$repo/releases/latest/download" } else { "https://github.com/$repo/releases/download/$version" }

$tmp = Join-Path ([IO.Path]::GetTempPath()) ("retake-" + [Guid]::NewGuid())
New-Item -ItemType Directory -Path $tmp | Out-Null
try {
    Write-Host "retake-install: downloading $asset ($version)"
    [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
    Invoke-WebRequest -UseBasicParsing -Uri "$base/$asset" -OutFile (Join-Path $tmp $asset)
    Invoke-WebRequest -UseBasicParsing -Uri "$base/$asset.sha256" -OutFile (Join-Path $tmp "$asset.sha256")

    $expected = ((Get-Content (Join-Path $tmp "$asset.sha256") -Raw).Split(' ')[0]).Trim().ToLower()
    $actual = (Get-FileHash -Algorithm SHA256 (Join-Path $tmp $asset)).Hash.ToLower()
    if ($expected -ne $actual) { throw "checksum mismatch for $asset" }

    Expand-Archive -Path (Join-Path $tmp $asset) -DestinationPath $tmp -Force
    New-Item -ItemType Directory -Path $installDir -Force | Out-Null
    Copy-Item (Join-Path $tmp "retake-$target\retake.exe") (Join-Path $installDir 'retake.exe') -Force

    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    if (-not (($userPath -split ';') -contains $installDir)) {
        [Environment]::SetEnvironmentVariable('Path', "$userPath;$installDir", 'User')
        $env:Path = "$env:Path;$installDir"
        Write-Host "retake-install: added $installDir to your PATH (restart your terminal to pick it up)"
    }
    $installed = & (Join-Path $installDir 'retake.exe') --version
    Write-Host "retake-install: installed $installed to $installDir"
}
finally {
    Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
}
