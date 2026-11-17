param([ValidateSet('check', 'full')][string]$Mode = 'full')

$ErrorActionPreference = 'Stop'
if ($env:OS -ne 'Windows_NT') { throw 'Run this script on Windows.' }
Set-Location (Split-Path $PSScriptRoot -Parent)

function Invoke-Checked([string]$File, [string[]]$Arguments) {
    $command = Get-Command $File -CommandType Application -ErrorAction Stop | Select-Object -First 1
    $ErrorActionPreference = 'Continue'
    $global:LASTEXITCODE = $null
    & $command.Source @Arguments 2>&1 | ForEach-Object { $_.ToString() }
    if ($null -eq $LASTEXITCODE) { throw "Failed to start $File." }
    if ($LASTEXITCODE -ne 0) {
        throw "$File $($Arguments -join ' ') failed with exit code $LASTEXITCODE"
    }
}

if (!$env:LIBCLANG_PATH) {
    $clang = @('C:\Program Files\LLVM\bin', 'C:\LLVM\bin') |
        Where-Object { Test-Path "$_\libclang.dll" } | Select-Object -First 1
    if (!$clang) { throw 'Install LLVM and set LIBCLANG_PATH to the directory containing libclang.dll.' }
    $env:LIBCLANG_PATH = $clang
}
if (!$env:GPUI_FXC_PATH) {
    $fxc = Get-ChildItem 'C:\Program Files (x86)\Windows Kits\10\bin\*\x64\fxc.exe' |
        Sort-Object FullName -Descending | Select-Object -First 1
    if (!$fxc) { throw 'Windows SDK shader compiler is missing.' }
    $env:GPUI_FXC_PATH = $fxc.FullName
}

Invoke-Checked 'cargo' @('fmt', '--all', '--', '--check')
Invoke-Checked 'cargo' @('test', '--workspace', '--locked')
Invoke-Checked 'cargo' @('clippy', '--workspace', '--all-targets', '--all-features', '--locked', '--', '-D', 'warnings')

if ($Mode -eq 'full') {
    $previous = $env:PEEP_TEST_WINDOWS_UI
    try {
        $env:PEEP_TEST_WINDOWS_UI = '1'
        Invoke-Checked 'cargo' @('test', '-p', 'peep-input', '--locked', '--test', 'windows_editor', '--', '--nocapture')
    } finally {
        $env:PEEP_TEST_WINDOWS_UI = $previous
    }
    Invoke-Checked 'python' @('script/test_inference.py', '--backend', 'onnx')
    Invoke-Checked 'python' @('script/package_desktop.py')
}
