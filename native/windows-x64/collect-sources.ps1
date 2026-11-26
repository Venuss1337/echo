# Obtain package source archives and record whether their recipe matches the
# build metadata. This inventory is evidence for review, not an approval.
param(
    [Parameter(Mandatory)][string] $ResourceRoot,
    [Parameter(Mandatory)][string] $SourceCache
)
$ErrorActionPreference = 'Stop'
$ResourceRoot = (Resolve-Path -LiteralPath $ResourceRoot).ProviderPath
$inventory = Get-Content "$ResourceRoot/inventory.json" -Raw | ConvertFrom-Json -AsHashtable
$sources = [IO.Path]::GetFullPath($SourceCache)
if ($sources -match '[^\x20-\x7e]') { throw 'Use an ASCII source cache for Windows tar.exe' }
New-Item -ItemType Directory -Force -Path $sources | Out-Null
$grouped = $inventory.packages.GetEnumerator() | Group-Object { $_.Value.source_archive }
$results = $grouped | ForEach-Object -Parallel {
    $entry = $_
    $file = Join-Path $using:sources (Split-Path $entry.Name -Leaf)
    $result = [ordered]@{
        url = $entry.Name
        packages = @($entry.Group | ForEach-Object Key)
        archive_sha256 = $null
        pkgbuild_sha256 = $null
        status = 'unverified'
    }
    try {
        if (!(Test-Path -LiteralPath $file)) {
            Invoke-WebRequest -Uri $entry.Name -OutFile $file -TimeoutSec 120 -ErrorAction Stop
        }
        $paths = & tar -tf $file 2>$null
        if ($LASTEXITCODE -ne 0) { throw 'Invalid source archive' }
        $recipe = @($paths | Where-Object { $_ -match '(^|/)PKGBUILD$' })
        if ($recipe.Count -ne 1 -or $recipe[0] -notmatch '^(?:[A-Za-z0-9_.+-]+/)*PKGBUILD$' -or $recipe[0].Split('/') -contains '..') {
            throw 'Invalid or ambiguous PKGBUILD path'
        }
        # Preserve exact recipe bytes when hashing, including any final newline.
        $recipeDir = Join-Path $using:sources ([guid]::NewGuid().ToString())
        New-Item -ItemType Directory -Path $recipeDir | Out-Null
        & tar -xf $file -C $recipeDir $recipe[0] 2>$null
        if ($LASTEXITCODE -ne 0) { throw 'Cannot extract PKGBUILD' }
        $result.pkgbuild_sha256 = (Get-FileHash -LiteralPath (Join-Path $recipeDir $recipe[0])).Hash.ToLowerInvariant()
        $result.archive_sha256 = (Get-FileHash -LiteralPath $file).Hash.ToLowerInvariant()
        $expected = @($entry.Group | ForEach-Object { $_.Value.pkgbuild_sha256 } | Sort-Object -Unique)
        $result.status = if ($expected.Count -eq 1 -and $expected[0] -ceq $result.pkgbuild_sha256) {
            'recipe-matches-binary-metadata'
        } else { 'recipe-mismatch' }
    } catch {
        $result.status = 'source-unavailable-or-unverified'
    }
    [pscustomobject] $result
} -ThrottleLimit 8
@($results | Sort-Object url) | ConvertTo-Json -Depth 5 -AsArray | Set-Content "$ResourceRoot/source-inventory.json"
if ($results.Where({ $_.status -ne 'recipe-matches-binary-metadata' }).Count) {
    throw 'Source evidence incomplete. See source-inventory.json'
}
