package app.tsumujiphone.local

import android.Manifest
import android.app.Service
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Binder
import android.os.Handler
import android.os.IBinder
import android.os.Looper
import android.os.SystemClock

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
  private lateinit var credentialStore: AndroidCredentialStore
  private var monitorEnabled = false
  private var hasRegistered = false
  private var initialDeadlineMillis: Long? = null
  private var recoveryStartedMillis: Long? = null
  private var nextRecoveryAttemptMillis: Long? = null
  private var recoveryError: String? = null
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

  override fun onBind(intent: Intent): IBinder = binder

  override fun onDestroy() {
    stopMonitor()
    super.onDestroy()
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
    return reconnectWithSavedSettings()
  }

  internal fun stopWaiting(): AndroidPlatformStatus {
    ensureNoActiveCall()
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

    val nativeError = NativeTelephony.nativeStart(server, userId, password, tailscaleAddress)
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
      }
      "failed", "stopped" -> {
        val error = NativeTelephony.nativeLastError()
          ?: if (hasRegistered) "SIP登録が失われました" else "SIP登録に失敗しました"
        if (hasRegistered) {
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
            NativeTelephony.nativeStop()
            setFailure("SIPサーバーから10秒以内に応答がありませんでした")
          }
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

  private companion object {
    val SERVER_PATTERN = Regex(
      """(?:\d{1,3}\.){3}\d{1,3}|(?:[A-Za-z0-9](?:[A-Za-z0-9-]{0,61}[A-Za-z0-9])?\.)*[A-Za-z0-9](?:[A-Za-z0-9-]{0,61}[A-Za-z0-9])?""",
    )
    val USER_ID_PATTERN = Regex("[A-Za-z0-9._~-]+")
    const val WAITING_PREFERENCES = "tsumuji_phone_waiting"
    const val WAITING_ENABLED = "waiting_enabled"
    const val SIP_SERVER = "sip_server"
    const val SIP_USER_ID = "sip_user_id"
    const val MONITOR_INTERVAL_MILLIS = 250L
    const val INITIAL_REGISTRATION_TIMEOUT_MILLIS = 10_000L
    const val RECOVERY_RETRY_INTERVAL_MILLIS = 5_000L
    const val RECOVERY_TIMEOUT_MILLIS = 60_000L
  }
}
