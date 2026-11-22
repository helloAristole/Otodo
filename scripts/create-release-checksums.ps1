param(
    [Parameter(Mandatory = $true)]
    [string]$BundleRoot,

    [Parameter(Mandatory = $true)]
    [string]$OutputPath
)

$ErrorActionPreference = "Stop"

if (-not (Test-Path -LiteralPath $BundleRoot -PathType Container)) {
    throw "Release bundle 目录不存在：$BundleRoot"
}

$expectedLocations = @(
    @{
        Directory = Join-Path $BundleRoot "nsis"
        Extension = ".exe"
    },
    @{
        Directory = Join-Path $BundleRoot "msi"
        Extension = ".msi"
    }
)

$assetsByName = [System.Collections.Generic.Dictionary[string, System.IO.FileInfo]]::new(
    [System.StringComparer]::OrdinalIgnoreCase
)

foreach ($location in $expectedLocations) {
    if (-not (Test-Path -LiteralPath $location.Directory -PathType Container)) {
        continue
    }

    $artifacts = Get-ChildItem -LiteralPath $location.Directory -File |
        Where-Object {
            [System.StringComparer]::OrdinalIgnoreCase.Equals(
                $_.Extension,
                $location.Extension
            )
        }

    foreach ($artifact in $artifacts) {
        if ($assetsByName.ContainsKey($artifact.Name)) {
            $existingPath = $assetsByName[$artifact.Name].FullName
            throw "Release 资产文件名重复（不区分大小写）：$($artifact.Name)；$existingPath；$($artifact.FullName)"
        }

        $assetsByName.Add($artifact.Name, $artifact)
    }
}

if ($assetsByName.Count -eq 0) {
    throw "未找到预期发布产物：nsis/*.exe 或 msi/*.msi（bundle：$BundleRoot）"
}

[string[]]$assetNames = @($assetsByName.Keys)
[Array]::Sort($assetNames, [System.StringComparer]::Ordinal)

$checksumLines = foreach ($assetName in $assetNames) {
    $artifact = $assetsByName[$assetName]
    $hash = (
        Get-FileHash -LiteralPath $artifact.FullName -Algorithm SHA256
    ).Hash.ToLowerInvariant()
    "$hash  $assetName"
}

$checksumContent = ($checksumLines -join "`n") + "`n"
$utf8WithoutBom = [System.Text.UTF8Encoding]::new($false)

if ([System.IO.Path]::IsPathRooted($OutputPath)) {
    $resolvedOutputPath = [System.IO.Path]::GetFullPath($OutputPath)
}
else {
    $currentDirectory = (Get-Location).ProviderPath
    $resolvedOutputPath = [System.IO.Path]::GetFullPath(
        (Join-Path $currentDirectory $OutputPath)
    )
}

$outputDirectory = [System.IO.Path]::GetDirectoryName($resolvedOutputPath)
[System.IO.Directory]::CreateDirectory($outputDirectory) | Out-Null
[System.IO.File]::WriteAllText(
    $resolvedOutputPath,
    $checksumContent,
    $utf8WithoutBom
)
