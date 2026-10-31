param(
    [string]$Serial,
    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"
$appDirectory = Split-Path $PSScriptRoot -Parent
if (-not $env:ANDROID_HOME) { $env:ANDROID_HOME = "$env:LOCALAPPDATA\Android\Sdk" }
$adb = Join-Path $env:ANDROID_HOME "platform-tools\adb.exe"
if (-not (Test-Path $adb)) { throw "Install Android SDK platform-tools first." }
if (-not $env:NDK_HOME) {
    $ndk = Get-ChildItem (Join-Path $env:ANDROID_HOME "ndk") -Directory |
        Sort-Object { [version]$_.Name } -Descending | Select-Object -First 1
    if (-not $ndk) { throw "Install an Android NDK using Android Studio SDK Manager." }
    $env:NDK_HOME = $ndk.FullName
}
${env:ORG_GRADLE_PROJECT_kotlin.incremental} = "false"

if (-not $Serial) {
    $phones = & $adb devices | Select-String "^(\S+)\s+device$" | ForEach-Object { $_.Matches[0].Groups[1].Value } |
        Where-Object { $_ -notlike "emulator-*" }
    if (@($phones).Count -ne 1) { throw "Connect exactly one phone with USB debugging on, or pass -Serial. Found: $($phones -join ', ')" }
    $Serial = @($phones)[0]
}

if (-not $SkipBuild) {
    # Phones are ARM; the emulator script builds x86_64 only.
    & npm.cmd --prefix $appDirectory run tauri -- android build --debug --target aarch64 --target armv7 --apk --ci
    if ($LASTEXITCODE -ne 0) { throw "Android build failed." }
}
$apk = Join-Path $appDirectory "src-tauri\gen\android\app\build\outputs\apk\universal\debug\app-universal-debug.apk"
if (-not (Test-Path $apk)) { throw "APK not found; run without -SkipBuild." }

& $adb -s $Serial install -r $apk
if ($LASTEXITCODE -ne 0) { throw "Install failed. Accept the prompt on the phone, or uninstall an older TrainLead signed with a different key." }
& $adb -s $Serial shell am start -n "app.trainlead.mobile.debug/app.trainlead.mobile.MainActivity"
if ($LASTEXITCODE -ne 0) { throw "TrainLead launch failed." }
Write-Host "TrainLead installed on $Serial. APK: $apk"
