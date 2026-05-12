package io.anuna.zetl.mobile

import android.Manifest
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Bundle
import android.os.Message
import android.util.Log
import android.webkit.ConsoleMessage
import android.webkit.GeolocationPermissions
import android.webkit.WebChromeClient
import android.webkit.WebResourceRequest
import android.webkit.WebSettings
import android.webkit.WebView
import android.webkit.WebViewClient
import androidx.activity.enableEdgeToEdge
import androidx.core.app.ActivityCompat
import androidx.core.content.ContextCompat

class MainActivity : TauriActivity() {
  private var pendingGeoOrigin: String? = null
  private var pendingGeoCallback: GeolocationPermissions.Callback? = null

  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
  }

  // SPEC-040: Tauri loads the WebView from `https://tauri.localhost/`
  // and the Rust setup() then navigates it to `http://127.0.0.1:23423/_mobile/...`.
  // The HTTPS → HTTP hop is mixed content; WebView's default
  // `MIXED_CONTENT_NEVER_ALLOW` blocks it and surfaces the failure as
  // the misleading `ERR_CLEARTEXT_NOT_PERMITTED`. Allow mixed content
  // here; the network-security-config still scopes actual cleartext
  // reach to 127.0.0.1 / localhost.
  //
  // Also wire up a WebChromeClient that handles geolocation prompts —
  // Tauri's default chrome client silently drops them so
  // `navigator.geolocation.watchPosition` times out and the user never
  // sees a permission dialog. We auto-grant the web prompt when the
  // user has already granted Android-level location, or request the
  // Android permission first and forward the result back to the web
  // callback.
  override fun onWebViewCreate(webView: WebView) {
    super.onWebViewCreate(webView)
    webView.settings.mixedContentMode = WebSettings.MIXED_CONTENT_ALWAYS_ALLOW
    webView.settings.setGeolocationEnabled(true)
    webView.settings.setSupportMultipleWindows(true)
    webView.settings.javaScriptCanOpenWindowsAutomatically = true

    webView.webChromeClient = object : WebChromeClient() {
      // Replacing Tauri's RustWebChromeClient also loses its
      // `onCreateWindow` handler, so target="_blank" links and
      // `window.open()` go nowhere. Route them out to the system
      // browser via an ACTION_VIEW intent — the standard pattern is a
      // throwaway WebView whose WebViewClient intercepts the first URL
      // load and fires the intent instead.
      override fun onCreateWindow(
        view: WebView,
        isDialog: Boolean,
        isUserGesture: Boolean,
        resultMsg: Message,
      ): Boolean {
        val tempWebView = WebView(view.context)
        tempWebView.webViewClient = object : WebViewClient() {
          override fun shouldOverrideUrlLoading(
            v: WebView,
            request: WebResourceRequest,
          ): Boolean {
            val intent = Intent(Intent.ACTION_VIEW, request.url).apply {
              flags = Intent.FLAG_ACTIVITY_NEW_TASK
            }
            try {
              this@MainActivity.startActivity(intent)
            } catch (e: Exception) {
              Log.w("zetl-webview", "failed to open external url", e)
            }
            return true
          }
        }
        val transport = resultMsg.obj as WebView.WebViewTransport
        transport.webView = tempWebView
        resultMsg.sendToTarget()
        return true
      }

      override fun onGeolocationPermissionsShowPrompt(
        origin: String?,
        callback: GeolocationPermissions.Callback?
      ) {
        val granted = ContextCompat.checkSelfPermission(
          this@MainActivity, Manifest.permission.ACCESS_FINE_LOCATION
        ) == PackageManager.PERMISSION_GRANTED ||
          ContextCompat.checkSelfPermission(
            this@MainActivity, Manifest.permission.ACCESS_COARSE_LOCATION
          ) == PackageManager.PERMISSION_GRANTED

        if (granted) {
          callback?.invoke(origin, true, false)
        } else {
          pendingGeoOrigin = origin
          pendingGeoCallback = callback
          ActivityCompat.requestPermissions(
            this@MainActivity,
            arrayOf(
              Manifest.permission.ACCESS_FINE_LOCATION,
              Manifest.permission.ACCESS_COARSE_LOCATION,
            ),
            GEO_PERM_REQUEST,
          )
        }
      }

      override fun onConsoleMessage(msg: ConsoleMessage?): Boolean {
        if (msg != null) {
          Log.d("zetl-webview", "${msg.sourceId()}:${msg.lineNumber()} ${msg.message()}")
        }
        return true
      }
    }
  }

  override fun onRequestPermissionsResult(
    requestCode: Int,
    permissions: Array<out String>,
    grantResults: IntArray,
  ) {
    super.onRequestPermissionsResult(requestCode, permissions, grantResults)
    if (requestCode == GEO_PERM_REQUEST) {
      val ok = grantResults.any { it == PackageManager.PERMISSION_GRANTED }
      pendingGeoCallback?.invoke(pendingGeoOrigin, ok, false)
      pendingGeoOrigin = null
      pendingGeoCallback = null
    }
  }

  companion object {
    private const val GEO_PERM_REQUEST = 4001
  }
}
