package app.tsumujiphone.local

import android.app.Activity
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.ServiceConnection
import android.os.IBinder
import android.webkit.WebView
import androidx.appcompat.app.AppCompatActivity
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin

@InvokeArg
internal class StartWaitingArgs {
  lateinit var server: String
  lateinit var userId: String
  var password: String? = null
}

@TauriPlugin
class TelephonyPlugin(private val activity: Activity) : Plugin(activity) {
  private var service: TelephonyService? = null
  private var bindingRequested = false
  private val pendingCalls = mutableListOf<(TelephonyService) -> Unit>()

  private val connection = object : ServiceConnection {
    override fun onServiceConnected(name: ComponentName, binder: IBinder) {
      service = (binder as TelephonyService.LocalBinder).service
      val connectedService = service ?: return
      pendingCalls.toList().forEach { call -> call(connectedService) }
      pendingCalls.clear()
    }

    override fun onServiceDisconnected(name: ComponentName) {
      service = null
    }
  }

  override fun load(webView: WebView) {
    val intent = Intent(activity, TelephonyService::class.java)
    bindingRequested = activity.bindService(intent, connection, Context.BIND_AUTO_CREATE)
  }

  override fun onDestroy(activity: AppCompatActivity) {
    if (bindingRequested) {
      activity.unbindService(connection)
    }
    bindingRequested = false
    service = null
    pendingCalls.clear()
  }

  @Command
  fun getStatus(invoke: Invoke) {
    withService(invoke) { connectedService ->
      invoke.resolve(connectedService.getStatus().toJson())
    }
  }

  @Command
  fun startWaiting(invoke: Invoke) {
    val args = invoke.parseArgs(StartWaitingArgs::class.java)
    val mainActivity = activity as? MainActivity
    if (mainActivity == null) {
      invoke.reject("マイク権限を確認できませんでした")
      return
    }
    mainActivity.requestAudioPermission { granted ->
      if (!granted) {
        invoke.reject("通話にはマイクの権限が必要です")
        return@requestAudioPermission
      }
      withService(invoke) { connectedService ->
        invoke.resolve(
          connectedService
            .startWaiting(args.server, args.userId, args.password)
            .toJson(),
        )
      }
    }
  }

  @Command
  fun stopWaiting(invoke: Invoke) {
    withService(invoke) { connectedService ->
      invoke.resolve(connectedService.stopWaiting().toJson())
    }
  }

  @Command
  fun resetCredentials(invoke: Invoke) {
    withService(invoke) { connectedService ->
      connectedService.resetCredentials()
      invoke.resolve()
    }
  }

  private fun withService(invoke: Invoke, operation: (TelephonyService) -> Unit) {
    val connectedService = service
    if (connectedService != null) {
      runCatching { operation(connectedService) }
        .onFailure { error -> invoke.reject(error.message ?: "Android通話基盤でエラーが発生しました") }
      return
    }
    if (!bindingRequested) {
      invoke.reject("Android通話Serviceへ接続できませんでした")
      return
    }
    pendingCalls += { pendingService ->
      runCatching { operation(pendingService) }
        .onFailure { error -> invoke.reject(error.message ?: "Android通話基盤でエラーが発生しました") }
    }
  }

  private fun AndroidPlatformStatus.toJson() = JSObject().apply {
    put("state", state.wireName)
    put("tailscaleAddress", tailscaleAddress)
    put("credentialStored", credentialStored)
    put("lastError", lastError)
  }
}
