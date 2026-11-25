# SPDX-License-Identifier: MPL-2.0
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$cache = Join-Path $root '.tools'
$data = Join-Path $root 'data'
New-Item -ItemType Directory -Force $cache, $data | Out-Null
$archive = Join-Path $cache 'msime-data.zip'
$sha256 = '2b68572feb962fc28a34b7946907732b9888ddcca3e53698667276617dd4373b'
$uri = 'https://github.com/metasequoiaime/msime/releases/download/msime-engine-capi-v0.1.2/msime-engine-data-quanpin-2.0.41.zip'

if (!(Test-Path -LiteralPath $archive) -or (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash -ne $sha256) {
    $download = Join-Path $cache 'msime-data.download'
    & curl.exe --fail --location --retry 3 --output $download $uri
    if ($LASTEXITCODE -ne 0) { throw 'Dictionary download failed.' }
    if ((Get-FileHash -LiteralPath $download -Algorithm SHA256).Hash -ne $sha256) {
        throw 'Dictionary checksum mismatch.'
    }
    Move-Item -LiteralPath $download -Destination $archive -Force
}

& tar.exe -xf $archive -C $data --strip-components 1
if ($LASTEXITCODE -ne 0) { throw 'Dictionary extraction failed.' }
Write-Output 'Dictionary 2.0.41 is ready.'
