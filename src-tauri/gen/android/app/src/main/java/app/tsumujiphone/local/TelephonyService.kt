package app.tsumujiphone.local

import android.Manifest
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.content.pm.ServiceInfo
import android.os.Binder
import android.os.Build
import android.os.Handler
import android.os.IBinder
import android.os.Looper
import android.os.SystemClock
import androidx.core.app.NotificationCompat
import androidx.core.app.ServiceCompat

internal enum class AndroidWaitingState(val wireName: String) {
  STOPPED("stopped"),
  PLATFORM_READY("platformReady"),
  REGISTERING("registering"),
  REGISTERED("registered"),
  ERROR("error"),
}

internal data class AndroidPlatformStatus(
  val state: AndroidWaitingState,
  val tailscaleAddress: String?,
  val credentialStored: Boolean,
  val savedServer: String?,
  val savedUserId: String?,
  val lastError: String?,
)

class TelephonyService : Service() {
  inner class LocalBinder : Binder() {
    val service: TelephonyService
      get() = this@TelephonyService
  }

  private val binder = LocalBinder()
  private val handler = Handler(Looper.getMainLooper())
  private val registrationMonitor = object : Runnable {
    override fun run() {
      observeRegistration()
      if (monitorEnabled) {
        handler.postDelayed(this, MONITOR_INTERVAL_MILLIS)
      }
    }
  }
  private val pushDeadline = Runnable { finishPushWake() }
  private lateinit var credentialStore: AndroidCredentialStore
  private lateinit var pushStore: PushStateStore
  private var monitorEnabled = false
  private var normalStartupAttempted = false
  private var hasRegistered = false
  private var initialDeadlineMillis: Long? = null
  private var recoveryStartedMillis: Long? = null
  private var nextRecoveryAttemptMillis: Long? = null
  private var recoveryError: String? = null
  private var pushWakeActive = false
  private var pushReceivedAt = 0L
  private var pushRegistrationRecorded = false
  private var status = AndroidPlatformStatus(
    state = AndroidWaitingState.STOPPED,
    tailscaleAddress = null,
    credentialStored = false,
    savedServer = null,
    savedUserId = null,
    lastError = null,
  )

  override fun onCreate() {
    super.onCreate()
    credentialStore = AndroidCredentialStore(applicationContext)
    pushStore = PushStateStore(applicationContext)
    createNotificationChannels()
    setStopped()
  }

  override fun onBind(intent: Intent): IBinder {
    ensureNormalStartup()
    return binder
  }

  override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
    if (intent?.action != ACTION_PUSH_WAKE) {
      stopSelf(startId)
      return START_NOT_STICKY
    }
    promotePushWakeToForeground()
    startPushWake(intent.getLongExtra(EXTRA_PUSH_RECEIVED_AT, System.currentTimeMillis()))
    return START_NOT_STICKY
  }

  override fun onDestroy() {
    handler.removeCallbacks(pushDeadline)
    stopMonitor()
    NativeTelephony.nativeStop()
    super.onDestroy()
  }

  override fun onTimeout(startId: Int, fgsType: Int) {
    finishPushWake()
  }

  internal fun getStatus(): AndroidPlatformStatus = status.withSavedSettings()

  internal fun startWaiting(
    server: String,
    userId: String,
    suppliedPassword: String?,
  ): AndroidPlatformStatus {
    validateServer(server)
    validateUserId(userId)
    ensureNoActiveCall()
    leavePushMode()

    val password = suppliedPassword?.takeIf(String::isNotEmpty)
      ?: credentialStore.loadPassword()
      ?: throw IllegalStateException("パスワードを入力してください")
    validatePassword(password)
    if (!suppliedPassword.isNullOrEmpty()) {
      credentialStore.savePassword(password)
    }

    val saved = waitingPreferences()
      .edit()
      .putBoolean(WAITING_ENABLED, true)
      .putString(SIP_SERVER, server)
      .putString(SIP_USER_ID, userId)
      .commit()
    check(saved) { "Androidの待受設定を保存できませんでした" }

    return beginInitialRegistration(server, userId, password)
  }

  internal fun reconnect(): AndroidPlatformStatus {
    ensureNoActiveCall()
    leavePushMode()
    return reconnectWithSavedSettings()
  }

  internal fun stopWaiting(): AndroidPlatformStatus {
    ensureNoActiveCall()
    leavePushMode()
    stopMonitor()
    val nativeError = NativeTelephony.nativeStop()
    val saved = waitingPreferences().edit().putBoolean(WAITING_ENABLED, false).commit()
    check(saved) { "Androidの待受設定を更新できませんでした" }
    setStopped()
    check(nativeError == null) { nativeError ?: "PJSIPを停止できませんでした" }
    return status
  }

  internal fun resetCredentials() {
    ensureNoActiveCall()
    leavePushMode()
    stopMonitor()
    NativeTelephony.nativeStop()
    credentialStore.deletePassword()
    val cleared = waitingPreferences().edit().clear().commit()
    check(cleared) { "Androidの待受設定を削除できませんでした" }
    status = AndroidPlatformStatus(
      state = AndroidWaitingState.STOPPED,
      tailscaleAddress = TailscaleAddressFinder.find(),
      credentialStored = false,
      savedServer = null,
      savedUserId = null,
      lastError = null,
    )
  }

  private fun ensureNormalStartup() {
    if (normalStartupAttempted || pushWakeActive) return
    normalStartupAttempted = true
    if (waitingPreferences().getBoolean(WAITING_ENABLED, false)) {
      runCatching { reconnectWithSavedSettings() }
        .onFailure { error ->
          if (status.state != AndroidWaitingState.ERROR) {
            setFailure(error.message ?: "SIP登録を開始できませんでした")
          }
        }
    } else {
      setStopped()
    }
  }

  private fun startPushWake(receivedAt: Long) {
    if (pushWakeActive) {
      return
    }
    val nativeState = runCatching { NativeTelephony.nativeRegistrationState() }
      .getOrDefault("stopped")
    if (nativeState == "registered" || nativeState == "registering") {
      pushStore.recordIgnored(receivedAt)
      removePushForeground()
      stopSelf()
      return
    }
    if (!pushStore.hasNotificationPermission() ||
      !waitingPreferences().getBoolean(WAITING_ENABLED, false)
    ) {
      pushStore.recordIgnored(receivedAt)
      removePushForeground()
      stopSelf()
      return
    }

    pushWakeActive = true
    pushReceivedAt = receivedAt
    pushRegistrationRecorded = false
    val remaining = (
      PUSH_LIFETIME_MILLIS -
        (System.currentTimeMillis() - receivedAt).coerceAtLeast(0L)
      ).coerceAtLeast(0L)
    handler.removeCallbacks(pushDeadline)
    handler.postDelayed(pushDeadline, remaining)
    runCatching { reconnectWithSavedSettings() }
      .onFailure { error ->
        if (pushWakeActive) {
          failPushWake(error.message ?: "SIP登録を開始できませんでした")
        }
      }
  }

  private fun reconnectWithSavedSettings(): AndroidPlatformStatus {
    val preferences = waitingPreferences()
    if (!preferences.getBoolean(WAITING_ENABLED, false)) {
      throw registrationFailure("待受が停止されています")
    }
    val server = preferences.getString(SIP_SERVER, null)
      ?: throw registrationFailure("SIPサーバーを入力してください")
    val userId = preferences.getString(SIP_USER_ID, null)
      ?: throw registrationFailure("ユーザIDを入力してください")
    val password = credentialStore.loadPassword()
      ?: throw registrationFailure("パスワードを入力してください")
    return beginInitialRegistration(server, userId, password)
  }

  private fun beginInitialRegistration(
    server: String,
    userId: String,
    password: String,
  ): AndroidPlatformStatus {
    stopMonitor()
    val stopError = NativeTelephony.nativeStop()
    if (stopError != null) {
      throw registrationFailure(stopError)
    }
    if (checkSelfPermission(Manifest.permission.RECORD_AUDIO) != PackageManager.PERMISSION_GRANTED) {
      throw registrationFailure("通話にはマイクの権限が必要です")
    }
    val tailscaleAddress = TailscaleAddressFinder.find()
      ?: throw registrationFailure(
        "TailscaleのIPv4アドレスを検出できません。Tailscaleへ接続してから再試行してください",
      )

    val pushDeviceId = PushStateStore(applicationContext).loadPairing()?.deviceId.orEmpty()
    val nativeError = NativeTelephony.nativeStart(
      server,
      userId,
      password,
      tailscaleAddress,
      pushDeviceId,
    )
    if (nativeError != null) {
      throw registrationFailure(nativeError, tailscaleAddress)
    }

    val now = SystemClock.elapsedRealtime()
    hasRegistered = false
    initialDeadlineMillis = now + INITIAL_REGISTRATION_TIMEOUT_MILLIS
    recoveryStartedMillis = null
    nextRecoveryAttemptMillis = null
    recoveryError = null
    status = AndroidPlatformStatus(
      state = AndroidWaitingState.REGISTERING,
      tailscaleAddress = tailscaleAddress,
      credentialStored = true,
      savedServer = server,
      savedUserId = userId,
      lastError = null,
    )
    startMonitor()
    return status
  }

  private fun observeRegistration() {
    if (!monitorEnabled) return

    val now = SystemClock.elapsedRealtime()
    when (NativeTelephony.nativeRegistrationState()) {
      "registered" -> {
        hasRegistered = true
        initialDeadlineMillis = null
        recoveryStartedMillis = null
        nextRecoveryAttemptMillis = null
        recoveryError = null
        status = status.copy(
          state = AndroidWaitingState.REGISTERED,
          tailscaleAddress = TailscaleAddressFinder.find(),
          lastError = null,
        )
        if (pushWakeActive && !pushRegistrationRecorded) {
          pushRegistrationRecorded = true
          pushStore.recordRegistered(pushReceivedAt, System.currentTimeMillis())
        }
      }
      "failed", "stopped" -> {
        val error = NativeTelephony.nativeLastError()
          ?: if (hasRegistered) "SIP登録が失われました" else "SIP登録に失敗しました"
        if (pushWakeActive) {
          failPushWake(error)
        } else if (hasRegistered) {
          recoverRegistration(now, error)
        } else {
          NativeTelephony.nativeStop()
          setFailure(error)
        }
      }
      else -> {
        val initialDeadline = initialDeadlineMillis
        val recoveryStarted = recoveryStartedMillis
        when {
          initialDeadline != null && now >= initialDeadline -> {
            if (pushWakeActive) {
              failPushWake("SIPサーバーから10秒以内に応答がありませんでした")
            } else {
              NativeTelephony.nativeStop()
              setFailure("SIPサーバーから10秒以内に応答がありませんでした")
            }
          }
          pushWakeActive -> status = status.copy(
            state = AndroidWaitingState.REGISTERING,
            lastError = null,
          )
          recoveryStarted != null -> recoverRegistration(now, recoveryError)
          else -> status = status.copy(
            state = AndroidWaitingState.REGISTERING,
            lastError = null,
          )
        }
      }
    }
  }

  private fun recoverRegistration(now: Long, error: String?) {
    val started = recoveryStartedMillis ?: now.also {
      recoveryStartedMillis = it
      nextRecoveryAttemptMillis = it + RECOVERY_RETRY_INTERVAL_MILLIS
    }
    recoveryError = error ?: recoveryError

    if (now - started >= RECOVERY_TIMEOUT_MILLIS) {
      setFailure(
        recoveryError?.let { "SIP登録を60秒以内に復旧できませんでした: $it" }
          ?: "SIP登録を60秒以内に復旧できませんでした",
      )
      return
    }

    status = status.copy(
      state = AndroidWaitingState.REGISTERING,
      tailscaleAddress = TailscaleAddressFinder.find(),
      lastError = "SIP接続が失われました。再接続しています",
    )
    val nextAttempt = nextRecoveryAttemptMillis ?: return
    if (now >= nextAttempt) {
      NativeTelephony.nativeReregister()?.let { recoveryError = it }
      nextRecoveryAttemptMillis = now + RECOVERY_RETRY_INTERVAL_MILLIS
    }
  }

  private fun failPushWake(message: String) {
    if (!pushWakeActive) return
    val safeMessage = redactPushError(message)
    handler.removeCallbacks(pushDeadline)
    NativeTelephony.nativeStop()
    setFailure(safeMessage)
    pushStore.recordFailed(pushReceivedAt, safeMessage)
    pushWakeActive = false
    removePushForeground()
    showPushFailureNotification()
    stopSelf()
  }

  private fun finishPushWake() {
    if (!pushWakeActive) return
    handler.removeCallbacks(pushDeadline)
    stopMonitor()
    NativeTelephony.nativeStop()
    pushStore.recordCompleted(pushReceivedAt)
    pushWakeActive = false
    setStopped()
    removePushForeground()
    stopSelf()
  }

  private fun leavePushMode() {
    if (!pushWakeActive) return
    handler.removeCallbacks(pushDeadline)
    pushWakeActive = false
    removePushForeground()
  }

  private fun redactPushError(message: String): String {
    var redacted = message
    val preferences = waitingPreferences()
    listOfNotNull(
      preferences.getString(SIP_SERVER, null),
      preferences.getString(SIP_USER_ID, null),
      credentialStore.loadPassword(),
    )
      .filter(String::isNotEmpty)
      .forEach { secret -> redacted = redacted.replace(secret, "[非表示]") }
    return redacted.take(500)
  }

  private fun promotePushWakeToForeground() {
    val notification = NotificationCompat.Builder(this, PUSH_ACTIVE_CHANNEL)
      .setSmallIcon(android.R.drawable.stat_sys_phone_call)
      .setContentTitle("Tsumuji Phone")
      .setContentText("着信接続中")
      .setContentIntent(mainActivityPendingIntent())
      .setOngoing(true)
      .setSilent(true)
      .setPriority(NotificationCompat.PRIORITY_LOW)
      .build()
    val serviceType = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
      ServiceInfo.FOREGROUND_SERVICE_TYPE_SHORT_SERVICE
    } else {
      0
    }
    ServiceCompat.startForeground(this, PUSH_NOTIFICATION_ID, notification, serviceType)
  }

  private fun removePushForeground() {
    ServiceCompat.stopForeground(this, ServiceCompat.STOP_FOREGROUND_REMOVE)
  }

  private fun showPushFailureNotification() {
    val notification = NotificationCompat.Builder(this, PUSH_FAILURE_CHANNEL)
      .setSmallIcon(android.R.drawable.stat_notify_error)
      .setContentTitle("Tsumuji Phone")
      .setContentText("着信接続に失敗しました")
      .setContentIntent(mainActivityPendingIntent())
      .setAutoCancel(true)
      .setPriority(NotificationCompat.PRIORITY_DEFAULT)
      .build()
    getSystemService(NotificationManager::class.java)
      .notify(PUSH_FAILURE_NOTIFICATION_ID, notification)
  }

  private fun mainActivityPendingIntent(): PendingIntent =
    PendingIntent.getActivity(
      this,
      0,
      Intent(this, MainActivity::class.java)
        .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP),
      PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
    )

  private fun createNotificationChannels() {
    val manager = getSystemService(NotificationManager::class.java)
    manager.createNotificationChannel(
      NotificationChannel(
        PUSH_ACTIVE_CHANNEL,
        "Push着信接続",
        NotificationManager.IMPORTANCE_LOW,
      ).apply {
        description = "Push受信後にSIPへ一時接続している間だけ表示します"
        setSound(null, null)
        enableVibration(false)
      },
    )
    manager.createNotificationChannel(
      NotificationChannel(
        PUSH_FAILURE_CHANNEL,
        "Push着信接続エラー",
        NotificationManager.IMPORTANCE_DEFAULT,
      ).apply {
        description = "Push受信後にSIPへ接続できなかった場合に表示します"
      },
    )
  }

  private fun startMonitor() {
    handler.removeCallbacks(registrationMonitor)
    monitorEnabled = true
    handler.post(registrationMonitor)
  }

  private fun stopMonitor() {
    monitorEnabled = false
    handler.removeCallbacks(registrationMonitor)
    initialDeadlineMillis = null
    recoveryStartedMillis = null
    nextRecoveryAttemptMillis = null
    recoveryError = null
  }

  private fun setStopped() {
    hasRegistered = false
    status = AndroidPlatformStatus(
      state = AndroidWaitingState.STOPPED,
      tailscaleAddress = TailscaleAddressFinder.find(),
      credentialStored = credentialStore.hasPassword(),
      savedServer = waitingPreferences().getString(SIP_SERVER, null),
      savedUserId = waitingPreferences().getString(SIP_USER_ID, null),
      lastError = null,
    )
  }

  private fun setFailure(message: String, tailscaleAddress: String? = TailscaleAddressFinder.find()) {
    stopMonitor()
    status = AndroidPlatformStatus(
      state = AndroidWaitingState.ERROR,
      tailscaleAddress = tailscaleAddress,
      credentialStored = credentialStore.hasPassword(),
      savedServer = waitingPreferences().getString(SIP_SERVER, null),
      savedUserId = waitingPreferences().getString(SIP_USER_ID, null),
      lastError = message,
    )
  }

  private fun registrationFailure(
    message: String,
    tailscaleAddress: String? = TailscaleAddressFinder.find(),
  ): IllegalStateException {
    setFailure(message, tailscaleAddress)
    return IllegalStateException(message)
  }

  private fun ensureNoActiveCall() {
    check(!NativeTelephony.nativeHasActiveCall()) {
      "通話中は待受設定を変更できません"
    }
  }

  private fun AndroidPlatformStatus.withSavedSettings(): AndroidPlatformStatus {
    val preferences = waitingPreferences()
    return copy(
      tailscaleAddress = TailscaleAddressFinder.find(),
      credentialStored = credentialStore.hasPassword(),
      savedServer = preferences.getString(SIP_SERVER, null),
      savedUserId = preferences.getString(SIP_USER_ID, null),
    )
  }

  private fun waitingPreferences() =
    getSharedPreferences(WAITING_PREFERENCES, MODE_PRIVATE)

  private fun validateServer(server: String) {
    require(server.length in 1..253 && SERVER_PATTERN.matches(server)) {
      "SIPサーバーにはIPv4アドレスまたはDNS/MagicDNSホスト名を入力してください"
    }
  }

  private fun validateUserId(userId: String) {
    require(userId.length in 1..32 && USER_ID_PATTERN.matches(userId)) {
      "ユーザIDは半角英数字または - . _ ~ の1～32文字で入力してください"
    }
  }

  private fun validatePassword(password: String) {
    require(password.length in 1..128 && password.all { it.code in 0x21..0x7e }) {
      "パスワードは空白を除くASCII文字1～128文字で入力してください"
    }
  }

  companion object {
    const val ACTION_PUSH_WAKE = "app.tsumujiphone.local.action.PUSH_WAKE"
    const val EXTRA_PUSH_RECEIVED_AT = "push_received_at"

    internal fun isWaitingEnabled(context: Context): Boolean =
      context.getSharedPreferences(WAITING_PREFERENCES, Context.MODE_PRIVATE)
        .getBoolean(WAITING_ENABLED, false)

    private val SERVER_PATTERN = Regex(
      """(?:\d{1,3}\.){3}\d{1,3}|(?:[A-Za-z0-9](?:[A-Za-z0-9-]{0,61}[A-Za-z0-9])?\.)*[A-Za-z0-9](?:[A-Za-z0-9-]{0,61}[A-Za-z0-9])?""",
    )
    private val USER_ID_PATTERN = Regex("[A-Za-z0-9._~-]+")
    private const val WAITING_PREFERENCES = "tsumuji_phone_waiting"
    private const val WAITING_ENABLED = "waiting_enabled"
    private const val SIP_SERVER = "sip_server"
    private const val SIP_USER_ID = "sip_user_id"
    private const val MONITOR_INTERVAL_MILLIS = 250L
    private const val INITIAL_REGISTRATION_TIMEOUT_MILLIS = 10_000L
    private const val RECOVERY_RETRY_INTERVAL_MILLIS = 5_000L
    private const val RECOVERY_TIMEOUT_MILLIS = 60_000L
    private const val PUSH_LIFETIME_MILLIS = 30_000L
    private const val PUSH_ACTIVE_CHANNEL = "push_wake_active"
    private const val PUSH_FAILURE_CHANNEL = "push_wake_failure"
    private const val PUSH_NOTIFICATION_ID = 51_001
    private const val PUSH_FAILURE_NOTIFICATION_ID = 51_002
  }
}
