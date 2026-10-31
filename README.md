# TrainLead
A local-first app for independent trainers: collect WhatsApp training requirements,
extract structured details, and show one cluster with every group and vendor sighting.

## Milestone Plan And Status

Work stops for a summary and verification at each milestone.

| Milestone | Deliverable | Status |
| --- | --- | --- |
| M1 | Rust core: models, export parser, LLM extraction, SQLite, dedup, tests | Implemented |
| M2 | Tokio/axum API, authenticated ingest, leads/CSV, headless CLI | Planned |
| M3 | Tauri 2 desktop UI, filters, source controls, notify folder watcher | Partial: local shell/UI |
| M4 | Tauri Android, Kotlin notification plugin, ACTION_SEND share target | Partial: emulator APK; integrations pending |
| M5 | Evaluate whatsapp-rust/wacore; opt-in read-only bridge; secure settings | Planned |
| M6 | Windows/macOS/Linux installers, signed debug APK, build documentation | Planned |

The existing backend, desktop, web, Android, bridge, and Capacitor files are legacy
Python/Electron/Capacitor prototypes, not the new Rust app. They are retained
unchanged and are not wired to the Rust workspace. Do not expose the legacy server
to the LAN: it does not authenticate all ingestion endpoints.
M1 remains a library. A Tauri local preview shell now runs on the Android emulator;
the HTTP server, full extraction workflow and production installers are not built yet.

## Android Emulator Preview

The `app/` directory contains the new Tauri 2 shell, connected directly to
trainlead-core. This brings a limited part of M3/M4 forward at the user's request;
it does not complete either milestone or replace the pending M2 API.

Available: on-device SQLite, first-run privacy notice, paste/export capture to a
visible review inbox, fixed demo ingestion through the real core, requirement
cards with full source sightings and logistics, search, repeated-only filtering,
start-date/latest sorting, light/dark themes and a mobile Capture shortcut.

**Live extraction uses Groq when `GROQ_API_KEY` is configured.** Paste and export
messages matching training keywords are sent to Groq and clustered; without a key, or
when Groq fails, they are saved as needs-review with the reason, never dropped.
Load demo uses fixed mock responses and visibly labels records DEMO; it never sends
messages to Groq. Loading demo again stores additional sightings.
Screenshot capture, keychain/Keystore, notifications, Android share target, watcher,
linked-device ingestion and full filters/CSV/manual-cluster UI remain pending.

Prerequisites: Rust 1.90+ for the shell (tested 1.99.1), Node 22+ (tested 24.16),
Java 17, Android SDK platform 36, build-tools, emulator and NDK (tested 29.0.13846066).
The existing PaisaPilot AVD is x86-64; the APK below is for x86-64 emulators only,
not ARM phones. Native Android project scaffolding lives under app/src-tauri/gen/android.

From the workspace root, on Windows PowerShell:

```powershell
npm.cmd --prefix app ci
rustup target add x86_64-linux-android
$env:ANDROID_HOME = "$env:LOCALAPPDATA\Android\Sdk"
$env:NDK_HOME = "$env:ANDROID_HOME\ndk\29.0.13846066"
npm.cmd --prefix app run tauri -- android init --ci --skip-targets-install
& .\app\scripts\run-android.ps1
```

Android init is only necessary when the generated project is missing. The script
starts PaisaPilot if needed, builds, installs and launches the debug-signed APK.
Use `-SkipBuild` to install/launch an existing APK, or `-SmokeTest` to run the
end-to-end WebView test (this accepts local consent and adds demo/review test data).
It does not wipe the emulator or existing app data. On a cold boot with SkipBuild,
Android may still be booting; retry after its home screen appears.

APK: `app/src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk`.
App ID: `app.trainlead.mobile.debug`. Data is in the app's private data directory,
not the legacy server database. Debug signing is Android's development key, not
a production release signature. Do not distribute this preview as a production app.

The run script sets `ORG_GRADLE_PROJECT_kotlin.incremental=false` to avoid Kotlin's
cross-drive incremental-cache failures when Cargo is on C: and this workspace is
on D:. Tauri's generated Gradle/Kotlin code still emits upstream deprecation warnings.
Vite excludes native files from its watcher to avoid Windows locked-library errors.

Desktop development: `npm.cmd --prefix app run tauri -- dev`.
Browser layout preview: `npm.cmd --prefix app run dev`, then http://127.0.0.1:1420/.
Browser preview does not have native storage commands. Android's bundled APK
needs neither that dev server nor an API server to run.

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

Rust edition 2024; core declared MSRV 1.88; shell minimum 1.90.
Verification uses Rust 1.99.1 on Windows and an x86-64 Android emulator.
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
- `extraction::LlmClient`: mockable async contract; `GroqClient` uses Groq's
    OpenAI-compatible chat completions API with JSON mode. Batches hold at most eight messages.
- `pipeline::Pipeline`: keyword prefilter, extraction/re-ask, validation, storage.
    Screenshots bypass the text filter and use a Groq vision model with the same schema.
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

`GroqClient::from_env()` reads `GROQ_API_KEY` (from https://console.groq.com/keys),
optional `GROQ_MODEL` (default `llama-3.3-70b-versatile`) and optional
`GROQ_VISION_MODEL` (default `meta-llama/llama-4-scout-17b-16e-instruct`). These model
IDs were not verified against a live Groq account; override them if Groq retires them.
The easiest setup, and the only one on Android: tap **AI extraction** in the Capture
panel, paste the key, and choose **Save and check key**. TrainLead verifies the key
with Groq (no message content is sent) and stores it in a file in the app's private
data directory, which the Android app sandbox protects. It is not yet Keystore- or
keychain-encrypted. A key saved in the app takes precedence over `.env`.
The desktop app also loads `.env` from its app-data directory
(`%APPDATA%\app.trainlead.mobile\.env` on Windows) and, in development, from the
workspace root; restart the app after changing it.
Messages already in the review inbox are extracted with **Retry extraction** (one
message) or **Retry all with Groq**; this processes them in place, with no duplicate
records. Screenshots in the inbox cannot be retried yet.
Tests require no credentials or network. No API key is checked into source, stored
in SQLite, or logged.

**On-device extraction (no key needed).** `rules::RuleExtractor` reads messages with
regular expressions and built-in skill and Indian-city vocabularies: skills, date
ranges ("2-7 Nov", "Nov 2 through 7", "28 Dec - 3 Jan", "Start Date: 1 November
2026"), payment dates, city/state, college names, mode, audience ("6th sem Engg"),
TFA / travel / food / stay, rates ("8k per day", "₹3,000 per day"), vendor name and
phone. It skips greetings, trainer-availability posts and non-training job ads, and
splits a message listing several cities into one requirement per city. It cannot
read screenshots and is less flexible than an LLM with unusual wording.
`rules::FallbackExtractor` tries Groq first and falls back to these rules whenever
Groq errors or returns output that fails validation; each result reports which
engine produced it. If Groq returns 404 for the configured model (retired or not on
the account), `GroqClient` lists the account's models, switches to an available chat
model (preferring the list in `PREFERRED_TEXT_MODELS`) and retries once.

Pasted text copied from WhatsApp (`[11:41 am, 31/10/2026] Sender: text`, several
messages at once) is split into separate messages before extraction, like an export.

HTTP 429/5xx responses get three retries with exponential backoff (1/2/4 seconds),
honoring numeric Retry-After values with a 60-second cap. Other HTTP failures return
an error. Groq JSON-mode rejections (`json_validate_failed`), invalid extraction JSON
or semantic validation failures get exactly one re-extraction; a second failure marks
all batch messages needs-review without inserting partial requirements. In the app,
transport/API failures also move the messages to needs-review with the error.

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

Prefiltered candidate messages and uploaded screenshots are sent to Groq when a
Groq key is configured. Local storage does not mean local inference. The first-run
notice discloses this and requires consent before the workspace opens.

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
HTTP retry timing, notification/share integrations and production packaging remain
unverified. The shell adds two tests for durable review capture and demo dedup.
The Android WebView smoke test checks the actual native IPC, demo cluster cards,
repeat/search filters, local capture and mobile width; it writes a generated
screenshot to app/.smoke/android-light.png. Browser checks cover consent and themes.

## Vendor And Trainer Onboarding (in the app)

The **Vendors** and **Trainers** tabs keep a local directory in `directory.db` in
the app data folder. Profiles move Draft -> Submitted -> Active/Verified or Rejected
(rejection needs a reason; editing a rejected profile returns it to Draft).

- Vendor: legal/display name, GSTIN (checked against PAN), PAN, address, PIN, website,
  contact person, WhatsApp numbers, domains. Required: company, contact, domains.
- Trainer: profile, languages, experience, day rate range, travel, modes, bio, skills
  (proficiency 1-5, years, last used, trainings delivered, evidence link), blocked
  dates. Required: profile, day rate, at least one skill, a resume document.
- Documents: PDF/PNG/JPEG up to 2 MB, checked by content, with optional expiry
  (expired certificates are flagged). Stored unencrypted in `documents/`.
- Only the reviewer can mark a skill verified; edits keep the reviewer's decision.
- "Seen in your WhatsApp requirements" suggests vendors from captured leads, matched
  by phone, and the vendor card counts its WhatsApp postings.

Limits: this is single-user and on-device; the phone owner acts as reviewer. Self
sign-up for other vendors/trainers needs the M8 server and accounts.

## Ecosystem Layer (E1-E8)

One real-world training need is one **Opportunity**; every vendor, WhatsApp group or
sender that posts it adds a **Posting**; the hand-offs form a **chain of custody**
(client -> vendor A -> vendor B -> trainer) with a rate on each hop. Code lives in
`crates/trainlead-marketplace/src/ecosystem/`, schema in `migrations/0002_*.sql`.

| Milestone | Status |
| --- | --- |
| E1 Opportunity/Posting/Chain, cluster import, merge/unmerge, client resolution | Implemented |
| E2-E8 trainer side, sourcing, engagements, delivery, settlement, reputation, docs | Planned |

E1 behavior:
- `ingest::import_clusters` turns each dedup cluster into an opportunity (status
  `Detected`) and each sighting into a posting. Re-importing is idempotent. A sender
  whose phone is a registered vendor contact maps to that vendor organization;
  everyone else becomes an external contact.
- The chain is **inferred** from posting order: client (or "unknown client") ->
  first poster -> next poster. Each inferred hop carries the upstream poster's
  advertised rate and is flagged `inferred`. Vendors add explicit hops with
  `chain::add_link` (re-post to a vendor, or offer to a trainer with the rate disclosed).
- Opportunity lifecycle `Detected -> Verified -> Open -> Sourcing -> Staffed ->
  InDelivery -> Completed -> Settled | Cancelled | Expired` is enforced in code
  (`Staffed -> Sourcing` is allowed when a trainer drops out). Every change is in the
  append-only `object_events` log.
- Merge moves every posting and chain hop to the target and records exactly what moved;
  unmerge restores it. Hops identical to one already on the target stay with the source.
  Unmerge must happen in reverse order of later merges.
- Client resolution: exact alias (after normalizing spellings such as Mysore/Mysuru,
  Engg/Engineering) is confirmed; a close fuzzy match is only **suggested** and an admin
  confirms or rejects it; a mismatched city never matches. Confirming adds the
  college text as a new alias.

Visibility rules implemented so far (`views::opportunity_view`; full matrix in E8):

| Viewer | Canonical fields | Postings | Chain hops | Rates |
| --- | --- | --- | --- | --- |
| Admin | yes | all, with contacts | all | all; each rate/contact read is audited |
| Vendor (involved) | yes | only its own; others as a count | hops it pays or receives | hops it pays; received hop only if disclosed |
| Trainer (offered) | yes, incl. logistics | none | only offers to them | their offered rate (audited) |
| Anyone else | no (403, audited) | no | no | no |

E1 assumptions: WhatsApp export timestamps without an offset are treated as IST and
stored as UTC; rates are parsed best-effort from text (`8k per day`, `1.5L total`),
with an unknown unit when none is written; opportunity confidence is the primary
sighting's field completeness (0-1). Vendor organizations and trainers have minimal
tables now; onboarding (M8/M9) extends them.

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
