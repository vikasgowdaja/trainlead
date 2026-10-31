# TrainLead
A local-first app for independent trainers: collect WhatsApp training requirements,
extract structured details, and show one cluster with every group and vendor sighting.

## Milestone Plan And Status

Work stops for a summary and verification at each milestone.

| Milestone | Deliverable | Status |
| --- | --- | --- |
| M1 | Rust core: models, export parser, LLM extraction, SQLite, dedup, tests | Implemented |
| M2 | Tokio/axum API, authenticated ingest, leads/CSV, headless CLI | Planned |
| M3 | Tauri 2 desktop UI, filters, source controls, notify folder watcher | Planned |
| M4 | Tauri Android, Kotlin notification plugin, ACTION_SEND share target | Planned |
| M5 | Evaluate whatsapp-rust/wacore; opt-in read-only bridge; secure settings | Planned |
| M6 | Windows/macOS/Linux installers, signed debug APK, build documentation | Planned |

The existing backend, desktop, web, Android, bridge, and Capacitor files are legacy
Python/Electron/Capacitor prototypes, not the new Rust app. They are retained
unchanged and are not wired to the Rust workspace. Do not expose the legacy server
to the LAN: it does not authenticate all ingestion endpoints.
M1 is a library, not a runnable UI, HTTP server, or APK. Installers are not built yet.

## M1 Build And Test

Install Rust through rustup and a native C/C++ build toolchain (on Windows, Visual
Studio Build Tools with Desktop development with C++ and a Windows SDK).
SQLite is bundled; a separate SQLite installation is not required.

From this workspace root:

```powershell
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo fmt --all -- --check
```

Rust edition 2024; declared MSRV 1.88. Verification uses Rust 1.99.1 on Windows.
The declared MSRV and other operating systems have not yet been tested.
Dependencies are pinned by the generated Cargo.lock.

## Core Interfaces

- `models`: strict requirement schema, normalized tech names, message metadata,
    sightings, clusters, and ingest counts. Optional extraction fields require
    explicit `null`; unknown fields, invalid enums/dates/phones are rejected.
- `parser::parse_export`: day-first Android/iOS exports, 12/24-hour timestamps,
    optional seconds, multiline messages, Unicode spacing, system-line boundaries.
    Invalid calendar dates return an error with the line number, not silent data loss.
- `source::MessageSource`: async ingestion contract. Paste, export and screenshot
    adapters are implemented. Folder/live/Android adapters arrive in M3-M5.
- `extraction::LlmClient`: mockable async contract; `AnthropicClient` uses the
    Messages API through reqwest/rustls. Batches contain at most eight messages.
- `pipeline::Pipeline`: keyword prefilter, extraction/re-ask, validation, storage.
    Screenshots bypass the text filter and use Claude vision with the same schema.
- `store::Store`: local SQLite with transactions, WAL, foreign keys, indexed exact
    hashes, every sighting, most-complete primary, group/repeat counts, manual merge
    and split, pending messages and a needs-review queue.
- `dedup::DedupThresholds`: runtime-configurable location (80/100), tech Jaccard
    (0.5) and raw-text (70/100) thresholds. Exact matches take precedence over similar.

Consumer flow: construct a `Store`, construct a `Pipeline` with an `LlmClient`, then
call `ingest_source` or `ingest`, passing today's local `chrono::NaiveDate`.
`Store::clusters()` returns primary data and full source records.
`messages_by_status("needs_review")` and `messages_by_status("pending")` expose
recoverable input; `screenshot(message_id)` retrieves retained image bytes.
Storage methods are synchronous; M2 must put SQLite work on a dedicated blocking
worker rather than doing expensive database work on Tokio request threads.

## LLM Configuration

`AnthropicClient::from_env()` reads `ANTHROPIC_API_KEY` and `CLAUDE_MODEL`.
The default model is `claude-sonnet-5-5`, as requested. This model identifier's
availability has not been verified against a live Anthropic account. Override it
with an available model when necessary. Tests require no credentials or network.
`AnthropicClient::new` accepts a secret supplied by the future keychain adapter.
No API key is checked into source, stored in SQLite, or logged by the core.

HTTP 429/5xx responses get three retries with exponential backoff (1/2/4 seconds),
honoring numeric Retry-After values with a 60-second cap. Other HTTP failures return
an error. Invalid extraction JSON or semantic validation failures get exactly one
re-extraction; a second failure marks all batch messages needs-review without
inserting partial requirements. Transport/API failures leave input pending.

## Explicit Assumptions

- Export dates are DD/MM/YY or DD/MM/YYYY (dots also accepted). Two-digit years
    mean 2000-2099. Ambiguous US month-first exports are not auto-detected.
- Export timestamps have no timezone: store ISO local wall time without inventing
    an offset. Live adapters should supply ISO timestamps with an offset.
- The prompt uses today's year for omitted years, or next year if the entire
    schedule has passed. New Year ranges cross into the following year. This is
    prompt guidance, not deterministic date extraction from arbitrary text.
- TFA means travel, food and accommodation. Unknown facts stay null/unknown;
    amounts remain strings so currency/unit/rate conditions are not discarded.
- Country code 91 is added by the prompt only for clearly Indian 10-digit numbers.
    Validation requires 7-15 digits but does not independently prove phone identity.
- Location is one signal: every comparable college/city field must pass its token
    set ratio; at least one must be comparable. Unknown dates/empty tech sets do not
    count as matches. Token set ratios use normalized Levenshtein via strsim.
- A similar cluster matches any stored sighting, so similarity can be transitive.
    Manual merge can intentionally override similarity; its non-exact added sources
    count as similar. Split recomputes counts, preserving all source records.
- Exact identity hashes a JSON tuple of normalized raw text, college, city and
    start date with SHA-256. Senders/vendors/groups do not affect exact identity.
- Screenshots accept JPEG/PNG/GIF/WebP up to 5 MiB. Original image bytes are retained
    locally for review; text sightings use the original message, not an LLM rewrite.

## Privacy And Safety

No telemetry. SQLite is local by default but **plaintext in M1**, including phone
numbers, messages and screenshots. Protect the database directory with OS account
permissions and disk encryption. OS keychain/Android Keystore integration and
evaluation of SQLCipher arrive with secure settings; application-level encryption
is not claimed in this milestone.

Prefiltered candidate messages and uploaded screenshots are sent to Anthropic when
the real client is used. Local storage does not mean local inference. The first-run
UI notice and explicit consent must explain this before any real ingestion in M3.

M2 will require an API key header on every ingest endpoint and default to loopback.
Linked-device ingestion will be opt-in and off by default: unofficial automation
can violate WhatsApp terms and get the number banned. Recommend a secondary SIM;
the bridge must be read-only and must never send. No linked device is connected by M1.
Android notification ingestion can miss muted groups and truncate long messages;
the settings screen and notification-access deep link are planned for M4.

## Verification Scope

The core suite covers both export formats, multiline/system boundaries, exact
forwards, similar vendor reposts, different city/date new clusters, three-city
messages, invalid JSON recovery/review, max-eight batching, keyword/LLM exclusions,
vision transcription/image retention, strict schema validation, manual merge/split,
transaction rollback, API-failure input retention, and on-disk persistence.

The AngularJS/Mysuru/2-7 Nov/6th sem/TFA/payment-25-Nov acceptance case uses a mocked
LLM response and checks normalization, structured fields and source retention.
It does **not** certify real model extraction accuracy. Live extraction, actual
HTTP retry timing, desktop/mobile integrations and packaging remain unverified.

## Legacy Prototype Reference

The following historical commands are not the Rust app's run/build instructions.
They are retained for reference only; do not use them for production deployment.

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
