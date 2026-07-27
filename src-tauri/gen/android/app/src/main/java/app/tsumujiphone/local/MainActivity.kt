package app.tsumujiphone.local

import android.Manifest
import android.content.pm.PackageManager
import android.os.Bundle
import android.webkit.WebView

class MainActivity : TauriActivity() {
  private var audioPermissionCallback: ((Boolean) -> Unit)? = null

  override fun onCreate(savedInstanceState: Bundle?) {
    if (BuildConfig.DEBUG) {
      WebView.setWebContentsDebuggingEnabled(true)
    }
    super.onCreate(savedInstanceState)
  }

  internal fun requestAudioPermission(callback: (Boolean) -> Unit) {
    if (checkSelfPermission(Manifest.permission.RECORD_AUDIO) == PackageManager.PERMISSION_GRANTED) {
      callback(true)
      return
    }
    audioPermissionCallback = callback
    requestPermissions(arrayOf(Manifest.permission.RECORD_AUDIO), AUDIO_PERMISSION_REQUEST)
  }

  override fun onRequestPermissionsResult(
    requestCode: Int,
    permissions: Array<out String>,
    grantResults: IntArray,
  ) {
    super.onRequestPermissionsResult(requestCode, permissions, grantResults)
    if (requestCode != AUDIO_PERMISSION_REQUEST) return
    val granted = grantResults.firstOrNull() == PackageManager.PERMISSION_GRANTED
    audioPermissionCallback?.invoke(granted)
    audioPermissionCallback = null
  }

  private companion object {
    const val AUDIO_PERMISSION_REQUEST = 41_001
  }
}
