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

@InvokeArg
internal class PairPushGatewayArgs {
  lateinit var gatewayUrl: String
  lateinit var pairingCode: String
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
  fun reconnect(invoke: Invoke) {
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
        invoke.resolve(connectedService.reconnect().toJson())
      }
    }
  }

  @Command
  fun resetCredentials(invoke: Invoke) {
    withService(invoke) { connectedService ->
      connectedService.resetCredentials()
      invoke.resolve()
    }
  }

  @Command
  fun requestPushPermission(invoke: Invoke) {
    val mainActivity = activity as? MainActivity
    if (mainActivity == null) {
      invoke.reject("通知権限を確認できませんでした")
      return
    }
    mainActivity.requestNotificationPermission {
      invoke.resolve()
    }
  }

  @Command
  fun openPushNotificationSettings(invoke: Invoke) {
    val mainActivity = activity as? MainActivity
    if (mainActivity == null) {
      invoke.reject("Androidの通知設定を開けませんでした")
      return
    }
    mainActivity.openNotificationSettings()
    invoke.resolve()
  }

  @Command
  fun openFullScreenIntentSettings(invoke: Invoke) {
    val mainActivity = activity as? MainActivity
    if (mainActivity == null) {
      invoke.reject("全画面着信の設定を開けませんでした")
      return
    }
    mainActivity.openFullScreenIntentSettings()
    invoke.resolve()
  }

  @Command
  fun pairPushGateway(invoke: Invoke) {
    val args = invoke.parseArgs(PairPushGatewayArgs::class.java)
    Thread {
      runCatching {
        PushGatewayClient(activity.applicationContext).pair(args.gatewayUrl, args.pairingCode)
      }.onSuccess {
        invoke.resolve()
      }.onFailure { error ->
        PushStateStore(activity.applicationContext).recordPairingFailure(
          error.message ?: "Push gatewayとのペアリングに失敗しました",
        )
        invoke.reject(error.message ?: "Push gatewayとのペアリングに失敗しました")
      }
    }.start()
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
    put("savedServer", savedServer)
    put("savedUserId", savedUserId)
    put("lastError", lastError)
    val push = PushStateStore(activity.applicationContext).status()
    put("notificationPermission", push.notificationPermission)
    put(
      "fullScreenIntentPermission",
      (activity as? MainActivity)?.canUseFullScreenIntent() == true,
    )
    put("pushFidReady", push.fidReady)
    put("pushPaired", push.paired)
    put("pushGatewayUrl", push.gatewayUrl)
    put("pushDeviceId", push.deviceId)
    put("pushPairingError", push.pairingError)
    put("pushSetupError", push.setupError)
    put("lastPushResult", push.lastResult)
    put("lastPushReceivedAt", push.lastReceivedAt)
    put("lastPushRegisteredAt", push.lastRegisteredAt)
    put("lastPushDurationMillis", push.lastDurationMillis)
    put("lastPushError", push.lastError)
  }
}
