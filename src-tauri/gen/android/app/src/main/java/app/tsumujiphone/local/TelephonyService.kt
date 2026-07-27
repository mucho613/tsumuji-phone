package app.tsumujiphone.local

import android.app.Service
import android.content.Intent
import android.os.Binder
import android.os.IBinder

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
  val lastError: String?,
)

class TelephonyService : Service() {
  inner class LocalBinder : Binder() {
    val service: TelephonyService
      get() = this@TelephonyService
  }

  private val binder = LocalBinder()
  private lateinit var credentialStore: AndroidCredentialStore
  private var status = AndroidPlatformStatus(
    state = AndroidWaitingState.STOPPED,
    tailscaleAddress = null,
    credentialStored = false,
    lastError = null,
  )

  override fun onCreate() {
    super.onCreate()
    credentialStore = AndroidCredentialStore(applicationContext)
    refreshStatus()
  }

  override fun onBind(intent: Intent): IBinder = binder

  internal fun getStatus(): AndroidPlatformStatus {
    refreshStatus()
    return status
  }

  internal fun startWaiting(
    server: String,
    userId: String,
    suppliedPassword: String?,
  ): AndroidPlatformStatus {
    validateServer(server)
    validateUserId(userId)

    val tailscaleAddress = TailscaleAddressFinder.find()
      ?: throw IllegalStateException(
        "TailscaleのIPv4アドレスを検出できません。Tailscaleへ接続してから再試行してください",
      )
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

    val nativeError = NativeTelephony.nativeStart(server, userId, password, tailscaleAddress)
    if (nativeError != null) {
      status = AndroidPlatformStatus(
        state = AndroidWaitingState.ERROR,
        tailscaleAddress = tailscaleAddress,
        credentialStored = true,
        lastError = nativeError,
      )
      throw IllegalStateException(nativeError)
    }
    status = AndroidPlatformStatus(
      state = AndroidWaitingState.REGISTERING,
      tailscaleAddress = tailscaleAddress,
      credentialStored = true,
      lastError = null,
    )
    return status
  }

  internal fun stopWaiting(): AndroidPlatformStatus {
    val nativeError = NativeTelephony.nativeStop()
    val saved = waitingPreferences().edit().putBoolean(WAITING_ENABLED, false).commit()
    check(saved) { "Androidの待受設定を更新できませんでした" }
    status = AndroidPlatformStatus(
      state = AndroidWaitingState.STOPPED,
      tailscaleAddress = TailscaleAddressFinder.find(),
      credentialStored = credentialStore.hasPassword(),
      lastError = null,
    )
    check(nativeError == null) { nativeError ?: "PJSIPを停止できませんでした" }
    return status
  }

  internal fun resetCredentials() {
    NativeTelephony.nativeStop()
    credentialStore.deletePassword()
    waitingPreferences().edit().putBoolean(WAITING_ENABLED, false).apply()
    status = AndroidPlatformStatus(
      state = AndroidWaitingState.STOPPED,
      tailscaleAddress = TailscaleAddressFinder.find(),
      credentialStored = false,
      lastError = null,
    )
  }

  private fun refreshStatus() {
    val tailscaleAddress = TailscaleAddressFinder.find()
    val waitingEnabled = waitingPreferences().getBoolean(WAITING_ENABLED, false)
    val credentialStored = credentialStore.hasPassword()
    status = when {
      waitingEnabled && tailscaleAddress == null -> AndroidPlatformStatus(
        state = AndroidWaitingState.ERROR,
        tailscaleAddress = null,
        credentialStored = credentialStored,
        lastError = "TailscaleのIPv4アドレスを検出できません",
      )
      waitingEnabled && credentialStored -> AndroidPlatformStatus(
        state = nativeWaitingState(),
        tailscaleAddress = tailscaleAddress,
        credentialStored = true,
        lastError = NativeTelephony.nativeLastError(),
      )
      else -> AndroidPlatformStatus(
        state = AndroidWaitingState.STOPPED,
        tailscaleAddress = tailscaleAddress,
        credentialStored = credentialStored,
        lastError = null,
      )
    }
  }

  private fun nativeWaitingState() = when (NativeTelephony.nativeRegistrationState()) {
    "registering" -> AndroidWaitingState.REGISTERING
    "registered" -> AndroidWaitingState.REGISTERED
    "failed" -> AndroidWaitingState.ERROR
    else -> AndroidWaitingState.PLATFORM_READY
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
  }
}
