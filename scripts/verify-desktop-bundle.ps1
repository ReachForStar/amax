param([switch]$RequireSignatures)

$ErrorActionPreference = 'Stop'

$projectRoot = Split-Path -Parent $PSScriptRoot
$config = Get-Content (Join-Path $projectRoot 'src-tauri/tauri.conf.json') -Raw -Encoding utf8 | ConvertFrom-Json
$version = $config.version
$releaseDir = Join-Path $projectRoot 'target/release'
$exePath = Join-Path $releaseDir 'amax.exe'
$msiDir = Join-Path $releaseDir 'bundle/msi'

if (-not (Test-Path -LiteralPath $exePath -PathType Leaf)) {
    throw "缺少构建后的程序：$exePath"
}

$exeVersion = (Get-Item -LiteralPath $exePath).VersionInfo.ProductVersion
if ($exeVersion -ne $version) {
    throw "程序版本 $exeVersion 与配置版本 $version 不一致"
}

# 安装包内的前端与命令会嵌入程序；检查发布程序中的关键功能内容。
$exeContent = [Text.Encoding]::UTF8.GetString([IO.File]::ReadAllBytes($exePath))
foreach ($marker in @('告警规则', 'get_alert_settings', 'set_alert_settings', 'get_alert_channels', '官网直连失败')) {
    if (-not $exeContent.Contains($marker)) {
        throw "程序缺少功能内容：$marker"
    }
}

if (-not (Test-Path -LiteralPath $msiDir -PathType Container)) {
    throw "缺少 MSI 目录：$msiDir"
}

$msiFiles = @(Get-ChildItem -LiteralPath $msiDir -Filter "*_$($version)_x64_*.msi" -File)
if ($msiFiles.Count -ne 2) {
    throw "版本 $version 应有中英文两个 MSI，实际 $($msiFiles.Count) 个"
}

$installer = New-Object -ComObject WindowsInstaller.Installer
try {
    foreach ($msi in $msiFiles) {
        if ($RequireSignatures -and -not (Test-Path -LiteralPath "$($msi.FullName).sig" -PathType Leaf)) {
            throw "缺少更新签名：$($msi.Name).sig"
        }
        $database = $installer.OpenDatabase($msi.FullName, 0)
        try {
            $view = $database.OpenView("SELECT Value FROM Property WHERE Property = 'ProductVersion'")
            try {
                $view.Execute()
                $record = $view.Fetch()
                if ($null -eq $record -or $record.StringData(1) -ne $version) {
                    throw "$($msi.Name) 的 ProductVersion 与 $version 不一致"
                }
            } finally {
                if ($null -ne $view) { [void][Runtime.InteropServices.Marshal]::ReleaseComObject($view) }
            }

            $fileView = $database.OpenView('SELECT FileName, Version FROM File')
            try {
                $fileView.Execute()
                $packagedExeVersion = $null
                while ($fileRecord = $fileView.Fetch()) {
                    if ($fileRecord.StringData(1) -eq 'amax.exe') {
                        $packagedExeVersion = $fileRecord.StringData(2)
                        break
                    }
                }
                if ($packagedExeVersion -ne "$version.0") {
                    throw "$($msi.Name) 内的 amax.exe 版本 $packagedExeVersion 与 $version 不一致"
                }
            } finally {
                if ($null -ne $fileView) { [void][Runtime.InteropServices.Marshal]::ReleaseComObject($fileView) }
            }
        } finally {
            if ($null -ne $database) { [void][Runtime.InteropServices.Marshal]::ReleaseComObject($database) }
        }
    }
} finally {
    [void][Runtime.InteropServices.Marshal]::ReleaseComObject($installer)
}

Write-Host "安装包核验通过：版本 $version；告警功能已嵌入程序；MSI：$($msiFiles.Name -join ', ')"
