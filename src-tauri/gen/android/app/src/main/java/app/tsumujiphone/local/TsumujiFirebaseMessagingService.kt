package app.tsumujiphone.local

import android.content.Intent
import androidx.core.content.ContextCompat
import com.google.firebase.messaging.FirebaseMessagingService
import com.google.firebase.messaging.RemoteMessage

class TsumujiFirebaseMessagingService : FirebaseMessagingService() {
  override fun onRegistered(installationId: String) {
    PushStateStore(applicationContext).saveFid(installationId)
    PushGatewayClient(applicationContext).refreshFidAsync(installationId)
  }

  override fun onMessageReceived(message: RemoteMessage) {
    val receivedAt = System.currentTimeMillis()
    val store = PushStateStore(applicationContext)
    val data = message.data
    if (message.priority != RemoteMessage.PRIORITY_HIGH ||
      data.keys != setOf(TYPE_KEY, CALL_TOKEN_KEY) ||
      data[TYPE_KEY] != WAKE_TYPE
    ) {
      return
    }
    if (!store.hasNotificationPermission()) {
      store.recordIgnored(receivedAt, "通知権限がないためPush待受を開始しませんでした")
      return
    }
    if (!TelephonyService.isWaitingEnabled(applicationContext)) {
      store.recordIgnored(receivedAt)
      return
    }
    val callToken = data[CALL_TOKEN_KEY] ?: return
    if (!store.acceptWakeToken(callToken, receivedAt)) {
      return
    }
    val nativeState = runCatching { NativeTelephony.nativeRegistrationState() }
      .getOrDefault("stopped")
    if (nativeState == "registered" || nativeState == "registering") {
      store.recordIgnored(receivedAt)
      return
    }

    store.recordReceived(receivedAt)
    val intent = Intent(applicationContext, TelephonyService::class.java)
      .setAction(TelephonyService.ACTION_PUSH_WAKE)
      .putExtra(TelephonyService.EXTRA_PUSH_RECEIVED_AT, receivedAt)
    runCatching {
      ContextCompat.startForegroundService(applicationContext, intent)
    }.onFailure { error ->
      store.recordFailed(
        receivedAt,
        error.message ?: "Androidの制限により着信接続を開始できませんでした",
      )
    }
  }

  private companion object {
    const val TYPE_KEY = "type"
    const val CALL_TOKEN_KEY = "callToken"
    const val WAKE_TYPE = "wake"
  }
}
