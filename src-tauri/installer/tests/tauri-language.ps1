# Select the first language from the generated installer, not the machine locale
# or a language left over from an earlier local build. Compatible with PS 5.1.
param(
    [Parameter(Mandatory = $true)][string]$TauriNsisDirectory,
    [Parameter(Mandatory = $true)][string]$NsisDirectory
)
$ErrorActionPreference = 'Stop'
$template = [IO.File]::ReadAllText((Join-Path $TauriNsisDirectory 'installer.nsi'))
$language = [regex]::Match($template, '(?m)^[ \t]*!insertmacro[ \t]+MUI_LANGUAGE[ \t]+"([A-Za-z][A-Za-z0-9]*)"').Groups[1].Value
if (-not $language) { throw 'Generated Tauri installer has no supported MUI_LANGUAGE declaration' }
foreach ($required in @(
    (Join-Path $TauriNsisDirectory 'utils.nsh'),
    (Join-Path $TauriNsisDirectory ($language + '.nsh')),
    (Join-Path $NsisDirectory ('Contrib\Language files\' + $language + '.nlf'))
)) {
    if (-not (Test-Path -LiteralPath $required -PathType Leaf)) { throw "Missing generated Tauri language dependency: $required" }
}
Write-Output $language
