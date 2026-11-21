$ErrorActionPreference = "Stop"

$Repo = "ridloabelian/saif-agent"
$InstallDir = if ($env:SAIF_INSTALL_DIR) { $env:SAIF_INSTALL_DIR } else { Join-Path $HOME ".saif\bin" }
$Version = if ($env:SAIF_VERSION) { $env:SAIF_VERSION } else { "latest" }
$Archive = "saif-x86_64-pc-windows-msvc.zip"

$BaseUrl = if ($Version -eq "latest") {
    "https://github.com/$Repo/releases/latest/download"
} else {
    "https://github.com/$Repo/releases/download/$Version"
}

$TmpDir = Join-Path ([System.IO.Path]::GetTempPath()) ("saif-install-" + [System.Guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Force -Path $TmpDir | Out-Null

try {
    $ZipPath = Join-Path $TmpDir $Archive
    $SumsPath = Join-Path $TmpDir "SHA256SUMS.txt"

    Write-Host "Downloading $Archive ($Version)..."
    Invoke-WebRequest -UseBasicParsing -Uri "$BaseUrl/$Archive" -OutFile $ZipPath
    Invoke-WebRequest -UseBasicParsing -Uri "$BaseUrl/SHA256SUMS.txt" -OutFile $SumsPath

    $ExpectedLine = Get-Content $SumsPath | Where-Object { $_ -match ("\s+\*?" + [regex]::Escape($Archive) + "$") } | Select-Object -First 1
    if (-not $ExpectedLine) {
        throw "Checksum entry for $Archive not found in SHA256SUMS.txt"
    }
    $Expected = ($ExpectedLine -split "\s+")[0].ToLowerInvariant()
    $Actual = (Get-FileHash $ZipPath -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($Actual -ne $Expected) {
        throw "SHA-256 mismatch: expected $Expected, got $Actual"
    }
    Write-Host "SHA-256 verified ($Actual)"

    New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
    Expand-Archive -Path $ZipPath -DestinationPath $InstallDir -Force

    $UserPath = [Environment]::GetEnvironmentVariable("Path", "User")
    if (-not $UserPath) { $UserPath = "" }
    $Parts = $UserPath -split ";" | Where-Object { $_ }
    if ($Parts -notcontains $InstallDir) {
        $NewPath = if ($UserPath) { "$UserPath;$InstallDir" } else { $InstallDir }
        [Environment]::SetEnvironmentVariable("Path", $NewPath, "User")
        Write-Host "Added $InstallDir to User PATH (restart terminal to apply)."
    }

    $SmokeHome = Join-Path $TmpDir "smoke"
    New-Item -ItemType Directory -Force -Path $SmokeHome | Out-Null
    $env:SAIF_HOME = $SmokeHome
    & (Join-Path $InstallDir "saif.exe") --version
} finally {
    Remove-Item -Recurse -Force -ErrorAction SilentlyContinue $TmpDir
}
