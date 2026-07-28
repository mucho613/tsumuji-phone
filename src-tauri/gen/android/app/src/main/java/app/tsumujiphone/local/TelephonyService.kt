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
import android.media.AudioAttributes
import android.media.AudioManager
import android.media.RingtoneManager
import android.net.Uri
import android.os.Binder
import android.os.Build
import android.os.Handler
import android.os.IBinder
import android.os.Looper
import android.os.SystemClock
import android.telecom.DisconnectCause
import android.util.Log
import androidx.core.app.NotificationCompat
import androidx.core.app.Person
import androidx.core.app.ServiceCompat
import androidx.core.telecom.CallAttributesCompat
import androidx.core.telecom.CallControlResult
import androidx.core.telecom.CallControlScope
import androidx.core.telecom.CallEndpointCompat
import androidx.core.telecom.CallsManager
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch
import kotlinx.coroutines.withTimeout

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

internal data class AndroidAudioRoute(
  val id: String,
  val name: String,
  val type: String,
)

internal data class AndroidAudioRouteState(
  val status: String,
  val current: AndroidAudioRoute?,
  val routes: List<AndroidAudioRoute>,
  val error: String?,
)

class TelephonyService : Service() {
  inner class LocalBinder : Binder() {
    val service: TelephonyService
      get() = this@TelephonyService
  }

  private val binder = LocalBinder()
  private val handler = Handler(Looper.getMainLooper())
  private val telecomCoroutineScope =
    CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
  private val registrationMonitor = object : Runnable {
    override fun run() {
      observeRegistration()
      if (monitorEnabled) {
        handler.postDelayed(this, MONITOR_INTERVAL_MILLIS)
      }
    }
  }
  private val pushDeadline = Runnable { finishPushWake() }
  private val audioRouteDeadline = Runnable {
    if (NativeTelephony.nativeHasActiveCall() && !audioRouteReady) {
      Log.w(
        TAG,
        "Audio route preparation timed out: current=${currentAudioEndpoint != null} " +
          "availableCount=${availableAudioEndpoints.size}",
      )
      audioRouteError = "音声経路を準備できませんでした"
    }
  }
  private lateinit var credentialStore: AndroidCredentialStore
  private lateinit var pushStore: PushStateStore
  private lateinit var callsManager: CallsManager
  private var monitorEnabled = false
  private var normalStartupAttempted = false
  private var uiBound = false
  private var hasRegistered = false
  private var initialDeadlineMillis: Long? = null
  private var recoveryStartedMillis: Long? = null
  private var nextRecoveryAttemptMillis: Long? = null
  private var recoveryError: String? = null
  private var pushWakeActive = false
  private var pushReceivedAt = 0L
  private var pushRegistrationRecorded = false
  private var telecomCallJob: Job? = null
  private var telecomCallControl: CallControlScope? = null
  private var telecomRegistrationFailed = false
  private var telecomDisconnectRequested = false
  private var telecomMarkedActive = false
  private var callForegroundActive = false
  private var pendingTelecomDisconnectCause: DisconnectCause? = null
  private var observedCallDirection = "none"
  private var observedCallPhase = "none"
  private var observedCallPeer = ""
  private var currentAudioEndpoint: CallEndpointCompat? = null
  private var availableAudioEndpoints = emptyList<CallEndpointCompat>()
  private var audioRouteReady = false
  private var audioRouteError: String? = null
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
    callsManager = CallsManager(applicationContext)
    callsManager.registerAppWithTelecom(CallsManager.CAPABILITY_BASELINE)
    createNotificationChannels()
    setStopped()
  }

  override fun onBind(intent: Intent): IBinder {
    uiBound = true
    ensureNormalStartup()
    return binder
  }

  override fun onUnbind(intent: Intent?): Boolean {
    uiBound = false
    return super.onUnbind(intent)
  }

  override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
    when (intent?.action) {
      ACTION_PUSH_WAKE -> {
        promotePushWakeToForeground()
        startPushWake(intent.getLongExtra(EXTRA_PUSH_RECEIVED_AT, System.currentTimeMillis()))
      }
      ACTION_ANSWER_CALL -> answerCallFromNotification()
      ACTION_REJECT_CALL -> rejectCallFromNotification()
      ACTION_HANG_UP_CALL -> hangUpCallFromNotification()
      ACTION_CALL_ACTIVE -> Unit
      else -> {
        stopSelf(startId)
        return START_NOT_STICKY
      }
    }
    return START_NOT_STICKY
  }

  override fun onDestroy() {
    handler.removeCallbacks(pushDeadline)
    handler.removeCallbacks(audioRouteDeadline)
    stopMonitor()
    cancelCallNotification()
    clearAudioRouteState()
    telecomCoroutineScope.cancel()
    stopNativeForServiceShutdown()
    super.onDestroy()
  }

  override fun onTimeout(startId: Int, fgsType: Int) {
    finishPushWake()
  }

  internal fun getStatus(): AndroidPlatformStatus = status.withSavedSettings()

  internal fun getAudioRouteState(
    bluetoothPermissionGranted: Boolean,
  ): AndroidAudioRouteState {
    if (!NativeTelephony.nativeHasActiveCall()) {
      return AndroidAudioRouteState("inactive", null, emptyList(), null)
    }
    val current = currentAudioEndpoint?.toAudioRoute(bluetoothPermissionGranted)
    val visibleRoutes = availableAudioEndpoints
      .filter { endpoint ->
        endpoint.type != CallEndpointCompat.TYPE_BLUETOOTH || bluetoothPermissionGranted
      }
      .filter { endpoint ->
        endpoint.type != CallEndpointCompat.TYPE_UNKNOWN ||
          endpoint.identifier == currentAudioEndpoint?.identifier
      }
      .map { endpoint -> endpoint.toAudioRoute(bluetoothPermissionGranted) }
      .toMutableList()
    if (currentAudioEndpoint?.type == CallEndpointCompat.TYPE_UNKNOWN &&
      visibleRoutes.none { route -> route.id == current?.id } &&
      current != null
    ) {
      visibleRoutes += current
    }
    val routeStatus = when {
      audioRouteReady -> "ready"
      audioRouteError != null -> "error"
      else -> "preparing"
    }
    return AndroidAudioRouteState(routeStatus, current, visibleRoutes, audioRouteError)
  }

  internal fun requestAudioRoute(
    routeId: String,
    completion: (String?) -> Unit,
  ) {
    val control = telecomCallControl
    if (control == null) {
      completion("音声経路はまだ準備中です")
      return
    }
    val endpoint = availableAudioEndpoints.firstOrNull {
      it.identifier.toString() == routeId
    }
    if (endpoint == null) {
      completion("選択した音声経路は現在利用できません")
      return
    }
    audioRouteError = null
    telecomCoroutineScope.launch {
      val error = runCatching {
        withTimeout(AUDIO_ROUTE_REQUEST_TIMEOUT_MILLIS) {
          when (control.requestEndpointChange(endpoint)) {
            is CallControlResult.Success -> null
            is CallControlResult.Error -> "音声経路を切り替えられませんでした"
          }
        }
      }.fold(
        onSuccess = { it },
        onFailure = { "音声経路の切り替えがタイムアウトしました" },
      )
      if (error != null) {
        audioRouteError = error
      }
      completion(error)
    }
  }

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

    observeNativeCall()
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

  private fun observeNativeCall() {
    val direction = NativeTelephony.nativeCallDirection()
    val phase = NativeTelephony.nativeCallPhase()
    val peer = NativeTelephony.nativeCallPeer().orEmpty()

    if (direction != observedCallDirection ||
      phase != observedCallPhase ||
      peer != observedCallPeer
    ) {
      Log.i(TAG, "Native call changed: direction=$direction phase=$phase")
    }
    if (direction != "none") {
      if (observedCallDirection == "none" ||
        (telecomCallJob == null &&
          telecomCallControl == null &&
          !telecomRegistrationFailed)
      ) {
        beginTelecomCall(direction, peer)
      }
      if (telecomCallControl != null &&
        (direction != observedCallDirection ||
        phase != observedCallPhase ||
        peer != observedCallPeer)
      ) {
        showCallNotification(direction, phase, peer)
      }
      if (phase == "active" && !callForegroundActive) {
        runCatching {
          promoteCallToForeground(direction, phase, peer)
        }.onFailure { error ->
          Log.e(TAG, "Active call foreground service could not be started", error)
        }
      }
      if (phase == "active" && !telecomMarkedActive) {
        markTelecomCallActive()
      }
    } else if (observedCallDirection != "none") {
      requestTelecomDisconnect(DisconnectCause(DisconnectCause.REMOTE))
      cancelCallNotification()
      clearAudioRouteState()
      if (!pushWakeActive && telecomCallJob == null) {
        stopSelf()
      }
    }

    observedCallDirection = direction
    observedCallPhase = phase
    observedCallPeer = peer
    if (direction == "none") {
      telecomRegistrationFailed = false
      telecomMarkedActive = false
    }
  }

  private fun beginTelecomCall(direction: String, peer: String) {
    if (telecomCallJob != null || telecomRegistrationFailed) {
      Log.w(
        TAG,
        "Core Telecom call was not started: jobActive=${telecomCallJob != null} " +
          "registrationFailed=$telecomRegistrationFailed",
      )
      return
    }

    prepareAudioRouteState()
    startService(
      Intent(this, TelephonyService::class.java).setAction(ACTION_CALL_ACTIVE),
    )
    telecomDisconnectRequested = false
    telecomMarkedActive = false
    pendingTelecomDisconnectCause = null
    val displayPeer = displayPeer(peer)
    val attributes = CallAttributesCompat(
      displayName = displayPeer,
      address = Uri.fromParts("sip", displayPeer, null),
      direction = if (direction == "incoming") {
        CallAttributesCompat.DIRECTION_INCOMING
      } else {
        CallAttributesCompat.DIRECTION_OUTGOING
      },
      callType = CallAttributesCompat.CALL_TYPE_AUDIO_CALL,
      callCapabilities = 0,
    )

    telecomCallJob = telecomCoroutineScope.launch {
      try {
        Log.i(TAG, "Adding $direction call to Core Telecom")
        callsManager.addCall(
          attributes,
          onAnswer = {
            promoteCallToForeground(
              "incoming",
              "connecting",
              NativeTelephony.nativeCallPeer().orEmpty(),
            )
            requireNativeCallSuccess(NativeTelephony.nativeAnswer())
          },
          onDisconnect = { cause ->
            disconnectNativeCall(cause)
          },
          onSetActive = {
            if (NativeTelephony.nativeCallPhase() == "ringing") {
              promoteCallToForeground(
                "incoming",
                "connecting",
                NativeTelephony.nativeCallPeer().orEmpty(),
              )
              requireNativeCallSuccess(NativeTelephony.nativeAnswer())
            }
          },
          onSetInactive = {
            throw UnsupportedOperationException("保留には対応していません")
          },
        ) {
          Log.i(TAG, "Core Telecom call control is ready")
          telecomCallControl = this
          launch {
            currentCallEndpoint.collect { endpoint ->
              Log.i(TAG, "Current audio endpoint updated: type=${endpoint.type}")
              currentAudioEndpoint = endpoint
              updateAudioRouteReadiness()
            }
          }
          launch {
            availableEndpoints.collect { endpoints ->
              Log.i(
                TAG,
                "Available audio endpoints updated: count=${endpoints.size} " +
                  "types=${endpoints.map { endpoint -> endpoint.type }.distinct().sorted()}",
              )
              availableAudioEndpoints = endpoints
              updateAudioRouteReadiness()
            }
          }
          showCallNotification(
            NativeTelephony.nativeCallDirection(),
            NativeTelephony.nativeCallPhase(),
            NativeTelephony.nativeCallPeer().orEmpty(),
          )
          if (direction == "outgoing") {
            promoteCallToForeground(
              direction,
              NativeTelephony.nativeCallPhase(),
              NativeTelephony.nativeCallPeer().orEmpty(),
            )
          }
          if (pushWakeActive) {
            removePushForeground()
          }
          pendingTelecomDisconnectCause?.let { cause ->
            disconnectTelecomCall(this, cause)
          }
        }
      } catch (_: CancellationException) {
        Log.i(TAG, "Core Telecom call control cancelled after disconnect")
      } catch (error: Exception) {
        telecomRegistrationFailed = true
        Log.e(TAG, "Core Telecom rejected the call", error)
      } finally {
        Log.i(TAG, "Core Telecom call control finished")
        telecomCallControl = null
        telecomCallJob = null
        telecomDisconnectRequested = false
        pendingTelecomDisconnectCause = null
        if (NativeTelephony.nativeCallDirection() == "none") {
          clearAudioRouteState()
          cancelCallNotification()
          if (!pushWakeActive) {
            stopSelf()
          }
        }
      }
    }
  }

  private fun answerCallFromNotification() {
    if (NativeTelephony.nativeCallDirection() != "incoming" ||
      NativeTelephony.nativeCallPhase() != "ringing"
    ) {
      return
    }
    val control = telecomCallControl ?: return
    val peer = NativeTelephony.nativeCallPeer().orEmpty()
    try {
      promoteCallToForeground("incoming", "connecting", peer)
    } catch (error: Exception) {
      Log.e(TAG, "Call foreground service could not be started", error)
      setFailure("バックグラウンド通話を開始できませんでした")
      return
    }
    telecomCoroutineScope.launch {
      when (control.answer(CallAttributesCompat.CALL_TYPE_AUDIO_CALL)) {
        is CallControlResult.Success -> {
          telecomMarkedActive = true
          val error = NativeTelephony.nativeAnswer()
          if (error != null) {
            NativeTelephony.nativeReject()
            requestTelecomDisconnect(DisconnectCause(DisconnectCause.ERROR))
            cancelCallNotification()
            setFailure(error)
          }
        }
        else -> {
          cancelCallForeground()
          showCallNotification("incoming", "ringing", peer)
        }
      }
    }
  }

  private fun rejectCallFromNotification() {
    if (NativeTelephony.nativeCallDirection() != "incoming") return
    NativeTelephony.nativeReject()
    requestTelecomDisconnect(DisconnectCause(DisconnectCause.REJECTED))
    cancelCallNotification()
  }

  private fun hangUpCallFromNotification() {
    if (!NativeTelephony.nativeHasActiveCall()) return
    NativeTelephony.nativeHangUp()
    requestTelecomDisconnect(DisconnectCause(DisconnectCause.LOCAL))
    cancelCallNotification()
  }

  private fun markTelecomCallActive() {
    val control = telecomCallControl ?: return
    telecomMarkedActive = true
    telecomCoroutineScope.launch {
      if (control.setActive() is CallControlResult.Error) {
        telecomMarkedActive = false
      }
    }
  }

  private fun requestTelecomDisconnect(cause: DisconnectCause) {
    if (telecomDisconnectRequested) return
    telecomDisconnectRequested = true
    pendingTelecomDisconnectCause = cause
    val control = telecomCallControl ?: return
    disconnectTelecomCall(control, cause)
  }

  private fun disconnectTelecomCall(
    control: CallControlScope,
    cause: DisconnectCause,
  ) {
    val callJob = telecomCallJob
    telecomCoroutineScope.launch {
      try {
        val result = withTimeout(TELECOM_DISCONNECT_TIMEOUT_MILLIS) {
          control.disconnect(cause)
        }
        if (result is CallControlResult.Error) {
          Log.w(TAG, "Core Telecom disconnect returned an error")
        }
      } catch (error: Exception) {
        Log.w(TAG, "Core Telecom disconnect did not complete normally", error)
      } finally {
        // Core Telecom may destroy the platform call without completing addCall().
        // Cancelling its owner prevents a stale call scope from blocking redial.
        callJob?.cancel()
      }
    }
  }

  private fun disconnectNativeCall(cause: DisconnectCause) {
    val error = if (cause.code == DisconnectCause.REJECTED &&
      NativeTelephony.nativeCallPhase() == "ringing"
    ) {
      NativeTelephony.nativeReject()
    } else {
      NativeTelephony.nativeHangUp()
    }
    requireNativeCallSuccess(error)
  }

  private fun requireNativeCallSuccess(error: String?) {
    if (error != null) {
      throw IllegalStateException(error)
    }
  }

  private fun prepareAudioRouteState() {
    handler.removeCallbacks(audioRouteDeadline)
    currentAudioEndpoint = null
    availableAudioEndpoints = emptyList()
    audioRouteReady = false
    audioRouteError = null
    handler.postDelayed(audioRouteDeadline, AUDIO_ROUTE_READY_TIMEOUT_MILLIS)
  }

  private fun updateAudioRouteReadiness() {
    if (currentAudioEndpoint != null && availableAudioEndpoints.isNotEmpty()) {
      audioRouteReady = true
      audioRouteError = null
      handler.removeCallbacks(audioRouteDeadline)
    }
  }

  private fun clearAudioRouteState() {
    handler.removeCallbacks(audioRouteDeadline)
    currentAudioEndpoint = null
    availableAudioEndpoints = emptyList()
    audioRouteReady = false
    audioRouteError = null
  }

  private fun CallEndpointCompat.toAudioRoute(
    bluetoothPermissionGranted: Boolean,
  ): AndroidAudioRoute {
    val routeType = when (type) {
      CallEndpointCompat.TYPE_EARPIECE -> "earpiece"
      CallEndpointCompat.TYPE_BLUETOOTH -> "bluetooth"
      CallEndpointCompat.TYPE_WIRED_HEADSET -> "wired"
      CallEndpointCompat.TYPE_SPEAKER -> "speaker"
      CallEndpointCompat.TYPE_STREAMING -> "streaming"
      else -> "unknown"
    }
    val fallbackName = when (routeType) {
      "earpiece" -> "受話口"
      "bluetooth" -> "Bluetooth"
      "wired" -> "有線ヘッドセット"
      "speaker" -> "スピーカー"
      "streaming" -> "外部デバイス"
      else -> "その他"
    }
    val endpointName = if (routeType == "bluetooth" && !bluetoothPermissionGranted) {
      fallbackName
    } else {
      name.toString().trim().takeIf(String::isNotEmpty) ?: fallbackName
    }
    return AndroidAudioRoute(identifier.toString(), endpointName, routeType)
  }

  private fun failPushWake(message: String) {
    if (!pushWakeActive) return
    val safeMessage = redactPushError(message)
    handler.removeCallbacks(pushDeadline)
    NativeTelephony.nativeStopDormant()
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
    if (NativeTelephony.nativeHasActiveCall()) {
      handler.postDelayed(pushDeadline, ACTIVE_CALL_CHECK_INTERVAL_MILLIS)
      return
    }
    stopMonitor()
    NativeTelephony.nativeStopDormant()
    pushStore.recordCompleted(pushReceivedAt)
    pushWakeActive = false
    setStopped()
    removePushForeground()
    stopSelf()
    if (uiBound) {
      normalStartupAttempted = false
      ensureNormalStartup()
    }
  }

  private fun leavePushMode() {
    if (!pushWakeActive) return
    handler.removeCallbacks(pushDeadline)
    pushWakeActive = false
    removePushForeground()
  }

  private fun stopNativeForServiceShutdown() {
    val keepPushBinding = waitingPreferences().getBoolean(WAITING_ENABLED, false) &&
      pushStore.loadPairing() != null
    if (keepPushBinding) {
      NativeTelephony.nativeStopDormant()
    } else {
      NativeTelephony.nativeStop()
    }
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
    if (callForegroundActive) {
      getSystemService(NotificationManager::class.java)
        .cancel(PUSH_NOTIFICATION_ID)
      return
    }
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

  private fun showCallNotification(direction: String, phase: String, peer: String) {
    val notification = buildCallNotification(direction, phase, peer)
    runCatching {
      if (callForegroundActive) {
        ServiceCompat.startForeground(
          this,
          CALL_NOTIFICATION_ID,
          notification,
          CALL_FOREGROUND_SERVICE_TYPES,
        )
      } else {
        getSystemService(NotificationManager::class.java)
          .notify(CALL_NOTIFICATION_ID, notification)
      }
      Log.i(
        TAG,
        "CallStyle notification posted: incoming=${direction == "incoming" && phase == "ringing"} " +
          "phase=$phase foreground=$callForegroundActive",
      )
    }.onFailure { error ->
      Log.e(TAG, "CallStyle notification could not be posted", error)
    }
  }

  private fun promoteCallToForeground(direction: String, phase: String, peer: String) {
    val notification = buildCallNotification(direction, phase, peer)
    ServiceCompat.startForeground(
      this,
      CALL_NOTIFICATION_ID,
      notification,
      CALL_FOREGROUND_SERVICE_TYPES,
    )
    callForegroundActive = true
    getSystemService(NotificationManager::class.java)
      .cancel(PUSH_NOTIFICATION_ID)
    Log.i(TAG, "Call foreground service started: direction=$direction phase=$phase")
  }

  private fun buildCallNotification(
    direction: String,
    phase: String,
    peer: String,
  ): android.app.Notification {
    val incoming = direction == "incoming" && phase == "ringing"
    val callerName = displayPeer(peer)
    val caller = Person.Builder()
      .setName(callerName)
      .setUri(Uri.fromParts("sip", callerName, null).toString())
      .setImportant(true)
      .build()
    val callStyle = if (incoming) {
      NotificationCompat.CallStyle.forIncomingCall(
        caller,
        callActionPendingIntent(ACTION_REJECT_CALL, CALL_ACTION_REJECT_REQUEST),
        callActionPendingIntent(ACTION_ANSWER_CALL, CALL_ACTION_ANSWER_REQUEST),
      )
    } else {
      NotificationCompat.CallStyle.forOngoingCall(
        caller,
        callActionPendingIntent(ACTION_HANG_UP_CALL, CALL_ACTION_HANG_UP_REQUEST),
      )
    }
    val channel = if (incoming) CALL_INCOMING_CHANNEL else CALL_ONGOING_CHANNEL
    val activityIntent = mainActivityPendingIntent()
    val notification = NotificationCompat.Builder(this, channel)
      .setSmallIcon(android.R.drawable.stat_sys_phone_call)
      .setContentTitle(if (incoming) "着信" else "通話中")
      .setContentText(callerName)
      .setContentIntent(activityIntent)
      .setFullScreenIntent(activityIntent, incoming)
      .setCategory(NotificationCompat.CATEGORY_CALL)
      .setPriority(NotificationCompat.PRIORITY_MAX)
      .setVisibility(NotificationCompat.VISIBILITY_PUBLIC)
      .setOngoing(true)
      .setOnlyAlertOnce(true)
      .setStyle(callStyle)
      .build()
    return notification
  }

  private fun cancelCallNotification() {
    cancelCallForeground()
    getSystemService(NotificationManager::class.java)
      .cancel(CALL_NOTIFICATION_ID)
  }

  private fun cancelCallForeground() {
    if (!callForegroundActive) return
    ServiceCompat.stopForeground(this, ServiceCompat.STOP_FOREGROUND_REMOVE)
    callForegroundActive = false
  }

  private fun callActionPendingIntent(action: String, requestCode: Int): PendingIntent =
    PendingIntent.getService(
      this,
      requestCode,
      Intent(this, TelephonyService::class.java).setAction(action),
      PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
    )

  private fun displayPeer(peer: String): String {
    val quotedName = Regex("\"([^\"]+)\"").find(peer)?.groupValues?.get(1)?.trim()
    if (!quotedName.isNullOrEmpty()) return quotedName
    val sipUser = Regex("""(?i)sip:([^@;>]+)""").find(peer)?.groupValues?.get(1)?.trim()
    if (!sipUser.isNullOrEmpty()) return sipUser
    return peer.trim().takeIf(String::isNotEmpty) ?: "発信者不明"
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
    val ringtone = RingtoneManager.getDefaultUri(RingtoneManager.TYPE_RINGTONE)
    val ringtoneAttributes = AudioAttributes.Builder()
      .setContentType(AudioAttributes.CONTENT_TYPE_SONIFICATION)
      .setLegacyStreamType(AudioManager.STREAM_RING)
      .setUsage(AudioAttributes.USAGE_NOTIFICATION_RINGTONE)
      .build()
    manager.createNotificationChannel(
      NotificationChannel(
        CALL_INCOMING_CHANNEL,
        "着信",
        NotificationManager.IMPORTANCE_HIGH,
      ).apply {
        description = "着信音と応答・拒否操作を表示します"
        enableVibration(true)
        setSound(ringtone, ringtoneAttributes)
      },
    )
    manager.createNotificationChannel(
      NotificationChannel(
        CALL_ONGOING_CHANNEL,
        "通話中",
        NotificationManager.IMPORTANCE_DEFAULT,
      ).apply {
        description = "通話中の切断操作を表示します"
        setSound(null, null)
        enableVibration(false)
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
    private const val TAG = "TsumujiTelephony"
    const val ACTION_PUSH_WAKE = "app.tsumujiphone.local.action.PUSH_WAKE"
    const val ACTION_ANSWER_CALL = "app.tsumujiphone.local.action.ANSWER_CALL"
    const val ACTION_REJECT_CALL = "app.tsumujiphone.local.action.REJECT_CALL"
    const val ACTION_HANG_UP_CALL = "app.tsumujiphone.local.action.HANG_UP_CALL"
    private const val ACTION_CALL_ACTIVE = "app.tsumujiphone.local.action.CALL_ACTIVE"
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
    private const val ACTIVE_CALL_CHECK_INTERVAL_MILLIS = 1_000L
    private const val AUDIO_ROUTE_READY_TIMEOUT_MILLIS = 5_000L
    private const val AUDIO_ROUTE_REQUEST_TIMEOUT_MILLIS = 5_000L
    private const val TELECOM_DISCONNECT_TIMEOUT_MILLIS = 5_000L
    private const val PUSH_ACTIVE_CHANNEL = "push_wake_active"
    private const val PUSH_FAILURE_CHANNEL = "push_wake_failure"
    private const val CALL_INCOMING_CHANNEL = "incoming_calls_v1"
    private const val CALL_ONGOING_CHANNEL = "ongoing_calls_v1"
    private const val PUSH_NOTIFICATION_ID = 51_001
    private const val PUSH_FAILURE_NOTIFICATION_ID = 51_002
    private const val CALL_NOTIFICATION_ID = 51_003
    private const val CALL_ACTION_ANSWER_REQUEST = 51_011
    private const val CALL_ACTION_REJECT_REQUEST = 51_012
    private const val CALL_ACTION_HANG_UP_REQUEST = 51_013
    private const val CALL_FOREGROUND_SERVICE_TYPES =
      ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE or
        ServiceInfo.FOREGROUND_SERVICE_TYPE_PHONE_CALL
  }
}
