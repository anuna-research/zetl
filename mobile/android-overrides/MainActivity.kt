package io.anuna.zetl.mobile

import android.os.Bundle
import android.webkit.WebSettings
import android.webkit.WebView
import androidx.activity.enableEdgeToEdge

class MainActivity : TauriActivity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
  }

  // SPEC-040: Tauri loads the WebView from `https://tauri.localhost/`
  // (the bundled `dist/` placeholder) and the Rust setup() then
  // navigates it to `http://127.0.0.1:23423/_mobile/...`. That
  // HTTPS → HTTP hop is mixed content; WebView's default
  // `MIXED_CONTENT_NEVER_ALLOW` blocks it and surfaces the failure as
  // the misleading `ERR_CLEARTEXT_NOT_PERMITTED` error.
  //
  // Allow mixed content here so the embedded serve is reachable. The
  // network-security-config still scopes actual cleartext reach to
  // `127.0.0.1` / `localhost` only.
  //
  // Implemented in MainActivity (not the auto-generated RustWebView)
  // because `cargo tauri android android-studio-script` regenerates
  // the `generated/` directory on every Gradle build, so patches to
  // RustWebView.kt get wiped before Kotlin compile.
  override fun onWebViewCreate(webView: WebView) {
    super.onWebViewCreate(webView)
    webView.settings.mixedContentMode = WebSettings.MIXED_CONTENT_ALWAYS_ALLOW
  }
}
