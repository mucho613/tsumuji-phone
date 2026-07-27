package app.tsumujiphone.local

import android.Manifest
import android.content.pm.PackageManager
import android.content.Intent
import android.net.Uri
import android.os.Bundle
import android.provider.Settings
import android.webkit.WebView
import com.google.firebase.messaging.FirebaseMessaging

class MainActivity : TauriActivity() {
  private var audioPermissionCallback: ((Boolean) -> Unit)? = null
  private var notificationPermissionCallback: ((Boolean) -> Unit)? = null

  override fun onCreate(savedInstanceState: Bundle?) {
    if (BuildConfig.DEBUG) {
      WebView.setWebContentsDebuggingEnabled(true)
    }
    super.onCreate(savedInstanceState)
    FirebaseMessaging.getInstance().register().addOnFailureListener {
      PushStateStore(applicationContext).recordSetupFailure()
    }
  }

  internal fun requestAudioPermission(callback: (Boolean) -> Unit) {
    if (checkSelfPermission(Manifest.permission.RECORD_AUDIO) == PackageManager.PERMISSION_GRANTED) {
      callback(true)
      return
    }
    audioPermissionCallback = callback
    requestPermissions(arrayOf(Manifest.permission.RECORD_AUDIO), AUDIO_PERMISSION_REQUEST)
  }

  internal fun requestNotificationPermission(callback: (Boolean) -> Unit) {
    if (android.os.Build.VERSION.SDK_INT < android.os.Build.VERSION_CODES.TIRAMISU ||
      checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) ==
      PackageManager.PERMISSION_GRANTED
    ) {
      callback(true)
      return
    }
    notificationPermissionCallback = callback
    requestPermissions(
      arrayOf(Manifest.permission.POST_NOTIFICATIONS),
      NOTIFICATION_PERMISSION_REQUEST,
    )
  }

  internal fun openNotificationSettings() {
    startActivity(
      Intent(Settings.ACTION_APP_NOTIFICATION_SETTINGS)
        .putExtra(Settings.EXTRA_APP_PACKAGE, packageName)
        .setData(Uri.parse("package:$packageName")),
    )
  }

  override fun onRequestPermissionsResult(
    requestCode: Int,
    permissions: Array<out String>,
    grantResults: IntArray,
  ) {
    super.onRequestPermissionsResult(requestCode, permissions, grantResults)
    val granted = grantResults.firstOrNull() == PackageManager.PERMISSION_GRANTED
    when (requestCode) {
      AUDIO_PERMISSION_REQUEST -> {
        audioPermissionCallback?.invoke(granted)
        audioPermissionCallback = null
      }
      NOTIFICATION_PERMISSION_REQUEST -> {
        PushStateStore(applicationContext).markNotificationPermissionRequested()
        notificationPermissionCallback?.invoke(granted)
        notificationPermissionCallback = null
      }
    }
  }

  private companion object {
    const val AUDIO_PERMISSION_REQUEST = 41_001
    const val NOTIFICATION_PERMISSION_REQUEST = 41_002
  }
}
