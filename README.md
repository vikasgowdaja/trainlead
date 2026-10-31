# TrainLead
Collects training requirements from WhatsApp groups, extracts tech stack, dates, college/location, travel/food/stay and vendor details, and merges exact and similar repeats.

## Run (desktop or server)
    cd backend && pip install -r requirements.txt
    export ANTHROPIC_API_KEY=sk-...  TL_KEY=choose-a-secret
    uvicorn app:app --host 0.0.0.0 --port 8000      # open http://localhost:8000
Desktop app: `cd desktop && npm i && npm start` (starts the backend and opens a window; `npx electron-builder` packages it).

## Four ways to get messages in
1. **Paste / Chat export / Screenshot**: use the panel in the app. Desktop auto-import: `python backend/watcher.py ~/Downloads` ingests every new export .txt.
2. **Share to app (Android)**: open the app URL over HTTPS in Chrome and choose Install app. TrainLead then appears in WhatsApp's Share menu (text, .txt exports, images). Tailscale or a Cloudflare Tunnel gives you HTTPS.
3. **Linked device (live)**: `cd bridge && npm i && API=http://127.0.0.1:8000 TL_KEY=... GROUPS="trainer,college" node index.js`, then scan the QR from a SECONDARY number's Linked devices screen. Unofficial, against WhatsApp's terms, ban risk exists. Never use your main number.
4. **Android notification listener**: see below.

## Android APK
    npm i @capacitor/core @capacitor/cli @capacitor/android && npx cap add android
    # edit capacitor.config.json server.url to your server, then:
    cp android/NotificationBridge.kt android/app/src/main/java/app/trainlead/
Add inside `<application>` of AndroidManifest.xml (and `android:usesCleartextTraffic="true"` if you use http):

    <service android:name=".NotificationBridge" android:exported="true"
      android:permission="android.permission.BIND_NOTIFICATION_LISTENER_SERVICE">
      <intent-filter><action android:name="android.service.notification.NotificationListenerService"/></intent-filter>
    </service>
Edit the `api` and `key` in NotificationBridge.kt, build in Android Studio, install, then enable Settings > Notification access > TrainLead.

## Duplicates
Exact: same message text and same college, city and start date. Similar: two of three signals (same start date, same college/city, 50% tech overlap) plus similar wording, or all three. Each cluster shows once with every group and vendor that posted it.
