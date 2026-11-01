package app.trainlead.mobile

import android.os.Bundle
import android.webkit.WebView
import androidx.activity.enableEdgeToEdge

class MainActivity : TauriActivity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
  }

  // The WebView only loads TrainLead's bundled pages, so the bridge is not exposed to the web.
  override fun onWebViewCreate(webView: WebView) {
    webView.addJavascriptInterface(NotificationBridge(this), "TrainLeadAndroid")
  }
}
