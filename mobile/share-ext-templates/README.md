# share-ext-templates

Drop-in source files that integrate the OS-native share affordances
with the SPEC-040 share-inbox file at
`app_data_dir/share-inbox.jsonl`. Use these **after** you've run
`cargo tauri ios init` / `cargo tauri android init` to generate the
platform project skeletons under `mobile/gen/`.

## Inbox file contract

Both platforms append JSONL entries to
`<container>/share-inbox.jsonl`, one entry per line:

```json
{"received_at":"<iso-8601-utc>","kind":"text|url|url_with_title","title":"...","body":"..."}
```

On launch, the Tauri shell calls `mobile_state::share_inbox_count()`
to decide whether to land the WebView on `/_mobile/capture?from=share`
(non-zero) or the usual `/_mobile/vaults` picker. The capture handler
calls `drain_share_inbox()` to read + delete the file.

## iOS — Share Extension target

1. **Add a new Share Extension target** in Xcode (opened from
   `mobile/gen/apple/<project>.xcworkspace` after
   `cargo tauri ios init`). Choose *Share Extension*; bundle id
   `io.anuna.zetl.mobile.share`.

2. **Configure an App Group**. In both the main app and the share
   extension target, add the same App Group capability, e.g.
   `group.io.anuna.zetl.mobile`. This gives both processes access to
   the shared container where the inbox file lives.

3. **Copy `ios/ShareViewController.swift`** into the share extension
   target. Update the `appGroupId` constant if you used a different
   App Group id.

4. **Update the main app's Rust shell to read from the App Group
   container** for `app_data_dir`. iOS file APIs route per-app data to
   the sandbox; for app-group access use
   `FileManager.containerURL(forSecurityApplicationGroupIdentifier:)`.
   Add a Tauri command that returns the container path, and have
   `mobile_state::set_app_data_dir` accept that path on iOS.

5. Build + sideload. Sharing text or a URL from Safari / Notes / etc.
   shows zetl-mobile in the share sheet; tapping it appends an entry
   to the inbox and dismisses. Re-launch the app — it lands on the
   capture form with the shared content prefilled.

## Android — Share-target Activity

1. **Add an `<intent-filter>` to the main Activity** in
   `mobile/gen/android/app/src/main/AndroidManifest.xml`:

   ```xml
   <activity
     android:name=".share.ShareReceiverActivity"
     android:exported="true">
     <intent-filter>
       <action android:name="android.intent.action.SEND" />
       <category android:name="android.intent.category.DEFAULT" />
       <data android:mimeType="text/plain" />
     </intent-filter>
   </activity>
   ```

2. **Copy `android/ShareReceiverActivity.kt`** into
   `mobile/gen/android/app/src/main/kotlin/io/anuna/zetl/mobile/share/`.
   It reads `ACTION_SEND` extras, appends to the inbox file at
   `filesDir/share-inbox.jsonl`, then launches the main Activity which
   the Tauri shell observes.

3. Build + run. Sharing text from Chrome / any app shows zetl-mobile
   in the share sheet; tapping it appends + launches.

## Desktop testing (no native code required)

The `/_mobile/share` HTTP endpoint accepts the same shape via form
POST — handy for testing the inbox flow without setting up the
platform targets:

```bash
curl -X POST \
  --data "title=From%20curl&body=A%20note%20I%20want%20to%20capture" \
  http://127.0.0.1:23423/_mobile/share
# → 303 See Other → /_mobile/capture?from=share
```

The capture form opens with both fields pre-filled.
