param(
    [string]$EmulatorName = "PaisaPilot",
    [switch]$SkipBuild,
    [switch]$SmokeTest
)

$ErrorActionPreference = "Stop"
$appDirectory = Split-Path $PSScriptRoot -Parent
if (-not $env:ANDROID_HOME) { $env:ANDROID_HOME = "$env:LOCALAPPDATA\Android\Sdk" }
$adb = Join-Path $env:ANDROID_HOME "platform-tools\adb.exe"
$emulator = Join-Path $env:ANDROID_HOME "emulator\emulator.exe"
if (-not (Test-Path $adb) -or -not (Test-Path $emulator)) { throw "Install Android SDK platform-tools and emulator first." }
if (-not $env:NDK_HOME) {
    $ndk = Get-ChildItem (Join-Path $env:ANDROID_HOME "ndk") -Directory |
        Sort-Object { [version]$_.Name } -Descending | Select-Object -First 1
    if (-not $ndk) { throw "Install an Android NDK using Android Studio SDK Manager." }
    $env:NDK_HOME = $ndk.FullName
}
${env:ORG_GRADLE_PROJECT_kotlin.incremental} = "false"
$devices = & $adb devices
if (-not ($devices | Select-String "^emulator-5554\s+device$")) {
    Start-Process -FilePath $emulator -ArgumentList @("-avd", $EmulatorName, "-port", "5554", "-gpu", "swiftshader", "-no-boot-anim", "-no-snapshot-load", "-no-snapshot-save")
}
if (-not $SkipBuild) {
    & npm.cmd --prefix $appDirectory run tauri -- android build --debug --target x86_64 --apk --ci
    if ($LASTEXITCODE -ne 0) { throw "Android build failed." }
}
$apk = Join-Path $appDirectory "src-tauri\gen\android\app\build\outputs\apk\universal\debug\app-universal-debug.apk"
if (-not (Test-Path $apk)) { throw "APK not found; run without -SkipBuild." }
& $adb -s emulator-5554 wait-for-device
if ($LASTEXITCODE -ne 0) { throw "Emulator did not connect." }
& $adb -s emulator-5554 install -r $apk
if ($LASTEXITCODE -ne 0) { throw "Install failed. If Android is still booting, rerun after the home screen appears." }
& $adb -s emulator-5554 shell am start -W -n "app.trainlead.mobile.debug/app.trainlead.mobile.MainActivity"
if ($LASTEXITCODE -ne 0) { throw "TrainLead launch failed." }
if ($SmokeTest) {
    $processId = (& $adb -s emulator-5554 shell pidof app.trainlead.mobile.debug).Trim()
    if (-not $processId) { throw "TrainLead process was not found." }
    & $adb -s emulator-5554 forward tcp:9223 "localabstract:webview_devtools_remote_$processId"
    if ($LASTEXITCODE -ne 0) { throw "WebView debug forwarding failed." }
    & npm.cmd --prefix $appDirectory run test:emulator
    if ($LASTEXITCODE -ne 0) { throw "Emulator smoke test failed." }
    & $adb -s emulator-5554 forward --remove tcp:9223
}
Write-Output "TrainLead is running in $EmulatorName. APK: $apk"