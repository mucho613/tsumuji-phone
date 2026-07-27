package app.tsumujiphone.local

import android.Manifest
import android.content.Context
import android.content.pm.PackageManager
import android.os.Build
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
import java.nio.charset.StandardCharsets
import java.security.KeyStore
import java.security.MessageDigest
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

internal data class PushStatus(
  val notificationPermission: String,
  val fidReady: Boolean,
  val paired: Boolean,
  val gatewayUrl: String?,
  val deviceId: String?,
  val pairingError: String?,
  val setupError: String?,
  val lastResult: String?,
  val lastReceivedAt: Long?,
  val lastRegisteredAt: Long?,
  val lastDurationMillis: Long?,
  val lastError: String?,
)

internal data class PushPairing(
  val gatewayUrl: String,
  val deviceId: String,
  val deviceCredential: String,
)

internal class PushStateStore(private val context: Context) {
  private val state = context.getSharedPreferences(STATE_PREFERENCES, Context.MODE_PRIVATE)
  private val secrets = context.getSharedPreferences(SECRET_PREFERENCES, Context.MODE_PRIVATE)

  fun status(): PushStatus = PushStatus(
    notificationPermission = notificationPermission(),
    fidReady = loadFid() != null,
    paired = loadPairing() != null,
    gatewayUrl = state.getString(GATEWAY_URL, null),
    deviceId = state.getString(DEVICE_ID, null),
    pairingError = state.getString(PAIRING_ERROR, null),
    setupError = state.getString(SETUP_ERROR, null),
    lastResult = state.getString(LAST_RESULT, null),
    lastReceivedAt = state.getLongOrNull(LAST_RECEIVED_AT),
    lastRegisteredAt = state.getLongOrNull(LAST_REGISTERED_AT),
    lastDurationMillis = state.getLongOrNull(LAST_DURATION_MILLIS),
    lastError = state.getString(LAST_ERROR, null),
  )

  fun notificationPermission(): String {
    if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU ||
      context.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) ==
      PackageManager.PERMISSION_GRANTED
    ) {
      return "granted"
    }
    return if (state.getBoolean(NOTIFICATION_PERMISSION_REQUESTED, false)) {
      "denied"
    } else {
      "notRequested"
    }
  }

  fun hasNotificationPermission(): Boolean = notificationPermission() == "granted"

  fun markNotificationPermissionRequested() {
    check(state.edit().putBoolean(NOTIFICATION_PERMISSION_REQUESTED, true).commit()) {
      "通知権限の状態を保存できませんでした"
    }
  }

  fun saveFid(fid: String) {
    require(fid.isNotBlank()) { "Firebase Installation IDが空です" }
    val encrypted = encrypt(fid, FID_KEY_ALIAS)
    check(
      secrets.edit()
        .putString(FID_CIPHERTEXT, encrypted.ciphertext)
        .putString(FID_IV, encrypted.iv)
        .commit(),
    ) {
      "Firebase Installation IDを保存できませんでした"
    }
    check(state.edit().remove(SETUP_ERROR).commit()) {
      "FCM準備状態を保存できませんでした"
    }
  }

  fun recordSetupFailure() {
    check(
      state.edit()
        .putString(SETUP_ERROR, "FCMの登録に失敗しました。Google Play開発者サービスと通信状態を確認してください")
        .commit(),
    ) {
      "FCM準備状態を保存できませんでした"
    }
  }

  fun loadFid(): String? = decrypt(
    secrets.getString(FID_CIPHERTEXT, null),
    secrets.getString(FID_IV, null),
    FID_KEY_ALIAS,
  )

  fun savePairing(gatewayUrl: String, deviceId: String, deviceCredential: String) {
    val encrypted = encrypt(deviceCredential, DEVICE_CREDENTIAL_KEY_ALIAS)
    check(
      secrets.edit()
        .putString(DEVICE_CREDENTIAL_CIPHERTEXT, encrypted.ciphertext)
        .putString(DEVICE_CREDENTIAL_IV, encrypted.iv)
        .commit(),
    ) {
      "端末資格情報を保存できませんでした"
    }
    check(
      state.edit()
        .putString(GATEWAY_URL, gatewayUrl)
        .putString(DEVICE_ID, deviceId)
        .remove(PAIRING_ERROR)
        .commit(),
    ) {
      "Push gateway設定を保存できませんでした"
    }
  }

  fun loadPairing(): PushPairing? {
    val gatewayUrl = state.getString(GATEWAY_URL, null) ?: return null
    val deviceId = state.getString(DEVICE_ID, null) ?: return null
    val credential = decrypt(
      secrets.getString(DEVICE_CREDENTIAL_CIPHERTEXT, null),
      secrets.getString(DEVICE_CREDENTIAL_IV, null),
      DEVICE_CREDENTIAL_KEY_ALIAS,
    ) ?: return null
    return PushPairing(gatewayUrl, deviceId, credential)
  }

  fun recordPairingFailure(message: String) {
    check(state.edit().putString(PAIRING_ERROR, message).commit()) {
      "Push gatewayエラーを保存できませんでした"
    }
  }

  fun clearPairingFailure() {
    check(state.edit().remove(PAIRING_ERROR).commit()) {
      "Push gateway状態を保存できませんでした"
    }
  }

  private fun decrypt(
    encodedCiphertext: String?,
    encodedIv: String?,
    keyAlias: String,
  ): String? = runCatching {
    encodedCiphertext ?: return null
    encodedIv ?: return null
    val key = keyStore().getKey(keyAlias, null) as? SecretKey ?: return null
    val cipher = Cipher.getInstance(CIPHER_TRANSFORMATION)
    cipher.init(
      Cipher.DECRYPT_MODE,
      key,
      GCMParameterSpec(GCM_TAG_LENGTH_BITS, Base64.decode(encodedIv, Base64.NO_WRAP)),
    )
    String(
      cipher.doFinal(Base64.decode(encodedCiphertext, Base64.NO_WRAP)),
      StandardCharsets.UTF_8,
    )
  }.getOrNull()

  private data class EncryptedValue(val ciphertext: String, val iv: String)

  private fun encrypt(value: String, keyAlias: String): EncryptedValue {
    val cipher = Cipher.getInstance(CIPHER_TRANSFORMATION)
    cipher.init(Cipher.ENCRYPT_MODE, getOrCreateKey(keyAlias))
    val ciphertext = cipher.doFinal(value.toByteArray(StandardCharsets.UTF_8))
    return EncryptedValue(
      Base64.encodeToString(ciphertext, Base64.NO_WRAP),
      Base64.encodeToString(cipher.iv, Base64.NO_WRAP),
    )
  }

  fun acceptWakeToken(token: String, nowMillis: Long): Boolean {
    if (!WAKE_TOKEN_PATTERN.matches(token)) return false
    val decoded = runCatching {
      Base64.decode(token, Base64.URL_SAFE or Base64.NO_WRAP or Base64.NO_PADDING)
    }.getOrNull() ?: return false
    if (decoded.size != 32) return false

    val tokenHash = MessageDigest.getInstance("SHA-256")
      .digest(token.toByteArray(StandardCharsets.US_ASCII))
      .joinToString("") { byte -> "%02x".format(byte) }
    synchronized(DEDUPE_LOCK) {
      val editor = state.edit()
      state.all
        .filterKeys { it.startsWith(DEDUPE_PREFIX) }
        .forEach { (key, value) ->
          val timestamp = value as? Long
          if (timestamp == null || nowMillis - timestamp >= DEDUPE_WINDOW_MILLIS) {
            editor.remove(key)
          }
        }
      val key = "$DEDUPE_PREFIX$tokenHash"
      val previous = state.getLong(key, 0L)
      if (previous > 0L && nowMillis - previous < DEDUPE_WINDOW_MILLIS) {
        editor.apply()
        return false
      }
      check(editor.putLong(key, nowMillis).commit()) {
        "Push重複検出情報を保存できませんでした"
      }
    }
    return true
  }

  fun recordReceived(receivedAt: Long) {
    updateResult("registering", receivedAt, null, null, null)
  }

  fun recordRegistered(receivedAt: Long, registeredAt: Long) {
    updateResult(
      "registered",
      receivedAt,
      registeredAt,
      (registeredAt - receivedAt).coerceAtLeast(0L),
      null,
    )
  }

  fun recordCompleted(receivedAt: Long) {
    val registeredAt = state.getLongOrNull(LAST_REGISTERED_AT)
    updateResult(
      "completed",
      receivedAt,
      registeredAt,
      registeredAt?.let { (it - receivedAt).coerceAtLeast(0L) },
      null,
    )
  }

  fun recordFailed(receivedAt: Long, message: String) {
    val now = System.currentTimeMillis()
    updateResult("failed", receivedAt, null, (now - receivedAt).coerceAtLeast(0L), message)
  }

  fun recordIgnored(receivedAt: Long, message: String? = null) {
    updateResult("ignored", receivedAt, null, 0L, message)
  }

  private fun updateResult(
    result: String,
    receivedAt: Long,
    registeredAt: Long?,
    durationMillis: Long?,
    error: String?,
  ) {
    val editor = state.edit()
      .putString(LAST_RESULT, result)
      .putLong(LAST_RECEIVED_AT, receivedAt)
      .remove(LAST_REGISTERED_AT)
      .remove(LAST_DURATION_MILLIS)
      .remove(LAST_ERROR)
    if (registeredAt != null) editor.putLong(LAST_REGISTERED_AT, registeredAt)
    if (durationMillis != null) editor.putLong(LAST_DURATION_MILLIS, durationMillis)
    if (error != null) editor.putString(LAST_ERROR, error)
    check(editor.commit()) { "Push処理結果を保存できませんでした" }
  }

  private fun android.content.SharedPreferences.getLongOrNull(key: String): Long? =
    if (contains(key)) getLong(key, 0L) else null

  private fun getOrCreateKey(keyAlias: String): SecretKey {
    val keyStore = keyStore()
    (keyStore.getKey(keyAlias, null) as? SecretKey)?.let { return it }
    return KeyGenerator
      .getInstance(KeyProperties.KEY_ALGORITHM_AES, ANDROID_KEYSTORE)
      .apply {
        init(
          KeyGenParameterSpec.Builder(
            keyAlias,
            KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT,
          )
            .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
            .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
            .setKeySize(256)
            .build(),
        )
      }
      .generateKey()
  }

  private fun keyStore(): KeyStore =
    KeyStore.getInstance(ANDROID_KEYSTORE).apply { load(null) }

  private companion object {
    val DEDUPE_LOCK = Any()
    val WAKE_TOKEN_PATTERN = Regex("[A-Za-z0-9_-]{43}")
    const val ANDROID_KEYSTORE = "AndroidKeyStore"
    const val CIPHER_TRANSFORMATION = "AES/GCM/NoPadding"
    const val GCM_TAG_LENGTH_BITS = 128
    const val FID_KEY_ALIAS = "app.tsumujiphone.local.fcm-fid"
    const val DEVICE_CREDENTIAL_KEY_ALIAS = "app.tsumujiphone.local.push-device"
    const val STATE_PREFERENCES = "tsumuji_phone_push_state"
    const val SECRET_PREFERENCES = "tsumuji_phone_push_secrets"
    const val FID_CIPHERTEXT = "fid_ciphertext"
    const val FID_IV = "fid_iv"
    const val DEVICE_CREDENTIAL_CIPHERTEXT = "device_credential_ciphertext"
    const val DEVICE_CREDENTIAL_IV = "device_credential_iv"
    const val GATEWAY_URL = "gateway_url"
    const val DEVICE_ID = "device_id"
    const val PAIRING_ERROR = "pairing_error"
    const val NOTIFICATION_PERMISSION_REQUESTED = "notification_permission_requested"
    const val LAST_RESULT = "last_result"
    const val LAST_RECEIVED_AT = "last_received_at"
    const val LAST_REGISTERED_AT = "last_registered_at"
    const val LAST_DURATION_MILLIS = "last_duration_millis"
    const val LAST_ERROR = "last_error"
    const val SETUP_ERROR = "setup_error"
    const val DEDUPE_PREFIX = "wake_hash_"
    const val DEDUPE_WINDOW_MILLIS = 120_000L
  }
}
